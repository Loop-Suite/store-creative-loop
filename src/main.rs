mod contact_sheet;
mod critique;
mod discover;
mod generation;
mod llm;
mod models;
mod quantify;
mod report;
mod spec;
mod state;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use llm::Llm;
use models::{PriorObservation, State};
use spec::Spec;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "storeloop",
    version,
    about = "Generate and refine real multi-device app-store screenshot sets"
)]
struct Cli {
    #[arg(long, default_value = "claude", global = true)]
    claude_bin: String,
    #[arg(long, global = true)]
    claude_model: Option<String>,
    #[arg(long, default_value_t = 1, global = true)]
    retries: u32,
    #[arg(long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run deterministic file and platform-policy gates only.
    Validate {
        #[arg(long)]
        spec: PathBuf,
        #[arg(long)]
        candidates: PathBuf,
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Validate, blind, independently critique, aggregate, and create an experiment handoff.
    Review {
        #[arg(long)]
        spec: PathBuf,
        #[arg(long)]
        candidates: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 3)]
        critics: usize,
    },
    /// Run a new round while tracking whether prior corroborated risks are re-observed.
    Refine {
        #[arg(long)]
        spec: PathBuf,
        #[arg(long)]
        candidates: PathBuf,
        #[arg(long)]
        prior: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 3)]
        critics: usize,
    },
    /// Generate real store PNGs, review variants, and feed the winner into the next round.
    Generate {
        #[arg(long)]
        spec: PathBuf,
        /// Raw captures live under RAW/<generation.source_target>/ in lexical frame order.
        #[arg(long)]
        raw: PathBuf,
        /// TTF, OTF, or supported TTC font used by the deterministic renderer.
        #[arg(long)]
        font: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 3)]
        variants: usize,
        #[arg(long, default_value_t = 2)]
        iterations: usize,
        #[arg(long, default_value_t = 3)]
        critics: usize,
        /// Segment id declared in [[generation.segments]]; defaults to the general audience.
        #[arg(long, default_value = "default")]
        segment: String,
    },
    /// Deterministically re-render an editable generation.json without another LLM call.
    Render {
        #[arg(long)]
        spec: PathBuf,
        #[arg(long)]
        raw: PathBuf,
        #[arg(long)]
        font: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Command::Validate {
            spec,
            candidates,
            json,
        } => validate(spec, candidates, json.as_deref()),
        Command::Review {
            spec,
            candidates,
            out,
            critics,
        } => run_review(&cli, spec, candidates, out, *critics, None),
        Command::Refine {
            spec,
            candidates,
            prior,
            out,
            critics,
        } => {
            let prior_state = state::load(prior)?;
            run_review(&cli, spec, candidates, out, *critics, Some(prior_state))
        }
        Command::Generate {
            spec,
            raw,
            font,
            out,
            variants,
            iterations,
            critics,
            segment,
        } => run_generation_loop(
            &cli,
            spec,
            raw,
            font,
            out,
            *variants,
            *iterations,
            *critics,
            segment,
        ),
        Command::Render {
            spec,
            raw,
            font,
            manifest,
            out,
        } => render_manifest(spec, raw, font, manifest, out),
    }
}

fn render_manifest(
    spec_path: &Path,
    raw_root: &Path,
    font_path: &Path,
    manifest_path: &Path,
    out: &Path,
) -> Result<()> {
    ensure_fresh_output(out)?;
    let spec = Spec::load(spec_path)?;
    let generation_spec = spec
        .generation
        .as_ref()
        .context("spec needs a [generation] section for the render command")?;
    let sources = generation::discover_raw_sources(raw_root, &spec, generation_spec)?;
    let manifest = generation::load_manifest(manifest_path)?;
    generation::render_plans(
        &spec,
        generation_spec,
        &manifest.plans,
        &sources,
        font_path,
        out,
    )?;
    println!(
        "rendered {} plans to {}",
        manifest.plans.len(),
        out.display()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_generation_loop(
    cli: &Cli,
    spec_path: &Path,
    raw_root: &Path,
    font_path: &Path,
    out: &Path,
    variants: usize,
    iterations: usize,
    critics: usize,
    segment_id: &str,
) -> Result<()> {
    anyhow::ensure!(
        variants >= 2,
        "generate requires at least two variants for blind review"
    );
    anyhow::ensure!(iterations > 0, "iterations must be greater than zero");
    anyhow::ensure!(critics > 0, "critics must be greater than zero");
    ensure_fresh_output(out)?;
    let spec = Spec::load(spec_path)?;
    let generation_spec = spec
        .generation
        .as_ref()
        .context("spec needs a [generation] section for the generate command")?;
    anyhow::ensure!(
        variants >= generation_spec.creative_families.len(),
        "generate needs at least one variant per creative family ({} configured)",
        generation_spec.creative_families.len()
    );
    let segment = spec.generation_segment(segment_id)?;
    let sources = generation::discover_raw_sources(raw_root, &spec, generation_spec)?;
    let generator = build_generator_llm(cli, generation_spec)?;
    std::fs::create_dir_all(out)?;

    let mut feedback: Option<String> = None;
    let mut final_winner: Option<String> = None;
    let mut final_candidates: Option<PathBuf> = None;
    let mut final_plan: Option<generation::CreativePlan> = None;

    for round in 1..=iterations {
        let round_dir = out.join(format!("round-{round:02}"));
        std::fs::create_dir_all(&round_dir)?;
        let raw_contact = round_dir.join("raw-contact.png");
        contact_sheet::render_sources(sources.primary(), 320, &raw_contact)?;
        eprintln!(
            "generation round {round}/{iterations}: provider={}, variants={variants}, segment={}",
            generator.provider_label, segment.id
        );
        let plans = generation::generate_plans(
            &generator,
            &spec,
            generation_spec,
            sources.primary(),
            &raw_contact,
            variants,
            round,
            &segment,
            feedback.as_deref(),
        )?;
        generation::write_manifest(
            &round_dir.join("generation.json"),
            round,
            &generator,
            &sources,
            &segment,
            &plans,
        )?;
        let candidates_root = round_dir.join("candidates");
        generation::render_plans(
            &spec,
            generation_spec,
            &plans,
            &sources,
            font_path,
            &candidates_root,
        )?;
        let review_dir = round_dir.join("review");
        run_review(cli, spec_path, &candidates_root, &review_dir, critics, None)?;
        let review_state = state::load(&review_dir.join("state.json"))?;
        let winner = review_state
            .quant
            .winner
            .clone()
            .context("generation round produced no eligible winner")?;
        let winning_plan = plans
            .iter()
            .find(|plan| plan.id == winner)
            .with_context(|| format!("winner {winner} has no generation plan"))?
            .clone();
        feedback = Some(generation_feedback(&review_state, &winning_plan)?);
        final_winner = Some(winner);
        final_candidates = Some(candidates_root);
        final_plan = Some(winning_plan);
    }

    let winner = final_winner.context("generation loop produced no winner")?;
    let candidates_root = final_candidates.context("generation loop produced no candidates")?;
    let winning_plan = final_plan.context("generation loop produced no plan")?;
    let final_dir = out.join("final");
    copy_tree(&candidates_root.join(&winner), &final_dir)?;
    std::fs::write(
        out.join("winner.json"),
        serde_json::to_vec_pretty(&winning_plan)?,
    )?;
    std::fs::write(
        out.join("summary.md"),
        format!(
            "# Generated store creative winner\n\n- Winner: `{winner}`\n- Segment: `{}`\n- Creative family: `{}`\n- Hypothesis: `{}` — {}\n- Iterations: {iterations}\n- Variants per round: {variants}\n- Final PNGs: [`final/`](final/)\n- Winning plan: [`winner.json`](winner.json)\n\nThe winner is an offline model-assisted recommendation. Use the final round's `review/experiment.md` before making a conversion claim.\n",
            segment.id,
            winning_plan.family.id(),
            winning_plan.hypothesis_id,
            winning_plan.hypothesis
        ),
    )?;
    println!("generated winner {winner}: {}", final_dir.display());
    Ok(())
}

fn build_generator_llm(cli: &Cli, generation: &spec::Generation) -> Result<Llm> {
    match generation.generator_backend.as_str() {
        "claude" => Ok(Llm::claude_cli(
            cli.claude_bin.clone(),
            cli.claude_model.clone(),
            cli.retries,
            cli.verbose,
        )),
        "openrouter" => {
            Llm::openrouter(generation.generator_model.clone(), cli.retries, cli.verbose)
        }
        backend => anyhow::bail!("unsupported generation backend: {backend}"),
    }
}

fn generation_feedback(state: &State, winner: &generation::CreativePlan) -> Result<String> {
    let criterion_means = state
        .quant
        .criterion_means
        .get(&winner.id)
        .cloned()
        .unwrap_or_default();
    Ok(format!(
        "Previous winner plan:\n{}\n\nWinner criterion means: {}\nCorroborated risks: {}\nMinority opinions: {}\nCreate new variants that preserve proven strengths while each tests one distinct response to these weaknesses. Do not merely paraphrase the previous copy.",
        serde_json::to_string_pretty(winner)?,
        serde_json::to_string(&criterion_means)?,
        serde_json::to_string(&state.quant.corroborated_risks)?,
        state.quant.minority_opinions.join("; ")
    ))
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    anyhow::ensure!(
        source.is_dir(),
        "winner directory does not exist: {}",
        source.display()
    );
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if entry.file_type()?.is_file() {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn validate(spec_path: &Path, candidates_root: &Path, json: Option<&Path>) -> Result<()> {
    let spec = Spec::load(spec_path)?;
    let candidates = discover::discover_candidates(candidates_root, &spec)?;
    for candidate in &candidates {
        println!(
            "{}: {}",
            candidate.id,
            if candidate.hard_pass {
                "PASS"
            } else {
                "BLOCKED"
            }
        );
        for issue in &candidate.policy_issues {
            println!(
                "  {} {} [{}] {}",
                issue.severity.label(),
                issue.code,
                issue.target_id,
                issue.evidence
            );
        }
    }
    if let Some(path) = json {
        anyhow::ensure!(
            !path.exists(),
            "refusing to overwrite validation JSON: {}",
            path.display()
        );
        std::fs::write(path, serde_json::to_vec_pretty(&candidates)?)
            .with_context(|| format!("failed to write validation JSON: {}", path.display()))?;
    }
    if candidates.iter().any(|candidate| !candidate.hard_pass) {
        anyhow::bail!("one or more candidates failed deterministic policy gates");
    }
    Ok(())
}

fn run_review(
    cli: &Cli,
    spec_path: &Path,
    candidates_root: &Path,
    out: &Path,
    critics: usize,
    prior: Option<State>,
) -> Result<()> {
    anyhow::ensure!(critics > 0, "critics must be greater than zero");
    ensure_fresh_output(out)?;
    let spec = Spec::load(spec_path)?;
    let candidates = discover::discover_candidates(candidates_root, &spec)?;
    let eligible = candidates
        .iter()
        .filter(|candidate| candidate.hard_pass)
        .cloned()
        .collect::<Vec<_>>();
    anyhow::ensure!(
        eligible.len() >= 2,
        "review requires at least two candidates that pass deterministic gates"
    );
    std::fs::create_dir_all(out)?;
    let (blind_map, sheets) =
        contact_sheet::build_contact_sheets(candidates_root, &eligible, &spec, out)?;
    let blind_ids = blind_map.keys().cloned().collect::<Vec<_>>();
    let claude = Llm::claude_cli(
        cli.claude_bin.clone(),
        cli.claude_model.clone(),
        cli.retries,
        cli.verbose,
    );
    let openrouter = match Llm::openrouter(
        spec.openrouter_critic_model.clone(),
        cli.retries,
        cli.verbose,
    ) {
        Ok(llm) => Some(llm),
        Err(error) => {
            if spec
                .critic_backends
                .iter()
                .any(|backend| backend == "openrouter")
            {
                eprintln!("warning: {error}; OpenRouter critic slots will use Claude (reduced provider diversity)");
            }
            None
        }
    };

    let mut discourse = Vec::new();
    for index in 0..critics {
        let requested = &spec.critic_backends[index % spec.critic_backends.len()];
        let selected = if requested == "openrouter" {
            openrouter.as_ref().unwrap_or(&claude)
        } else {
            &claude
        };
        let lens = &spec.lenses[index % spec.lenses.len()];
        eprintln!(
            "critic {}: provider={}, lens={}",
            index + 1,
            selected.provider_label,
            lens.id
        );
        discourse.push(critique::run_one(
            selected,
            &spec,
            lens,
            &blind_ids,
            &sheets,
            index + 1,
        )?);
    }
    critique::unblind(&mut discourse, &blind_map)?;
    let quant = quantify::quantify(&candidates, &discourse);
    let prior_observations = prior
        .as_ref()
        .map(|state| compare_prior(state, &quant.corroborated_risks))
        .unwrap_or_default();
    let round = prior.as_ref().map(|state| state.round + 1).unwrap_or(1);
    let state = State {
        round,
        spec_name: spec.name.clone(),
        generated_at_note: unix_timestamp_note(),
        blind_map,
        candidates,
        discourse,
        quant,
        prior_observations,
    };
    state::write(&out.join("state.json"), &state)?;
    report::write_reports(out, &spec, &state)?;
    println!(
        "wrote {}, {}, and {}",
        out.join("state.json").display(),
        out.join("report.md").display(),
        out.join("experiment.md").display()
    );
    Ok(())
}

fn compare_prior(prior: &State, current: &[models::CorroboratedRisk]) -> Vec<PriorObservation> {
    let key = |candidate: &str, category: &str, target: &str, frame: &str| {
        (
            candidate.to_string(),
            category.to_string(),
            target.to_string(),
            frame.to_string(),
        )
    };
    let current_keys = current
        .iter()
        .map(|risk| {
            key(
                &risk.candidate_id,
                &risk.category,
                &risk.target_id,
                &risk.frame,
            )
        })
        .collect::<BTreeSet<_>>();
    let prior_keys = prior
        .quant
        .corroborated_risks
        .iter()
        .map(|risk| {
            key(
                &risk.candidate_id,
                &risk.category,
                &risk.target_id,
                &risk.frame,
            )
        })
        .collect::<BTreeSet<_>>();
    prior_keys
        .union(&current_keys)
        .map(
            |(candidate_id, category, target_id, frame)| PriorObservation {
                candidate_id: candidate_id.clone(),
                category: category.clone(),
                target_id: target_id.clone(),
                frame: frame.clone(),
                status: if prior_keys.contains(&(
                    candidate_id.clone(),
                    category.clone(),
                    target_id.clone(),
                    frame.clone(),
                )) {
                    if current_keys.contains(&(
                        candidate_id.clone(),
                        category.clone(),
                        target_id.clone(),
                        frame.clone(),
                    )) {
                        "STILL_OPEN"
                    } else {
                        "NOT_REOBSERVED"
                    }
                } else {
                    "NEW"
                }
                .into(),
            },
        )
        .collect()
}

fn unix_timestamp_note() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{seconds}; generated locally; no network time authority asserted")
}

fn ensure_fresh_output(out: &Path) -> Result<()> {
    if !out.exists() {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(out)
        .with_context(|| format!("failed to inspect output directory: {}", out.display()))?;
    anyhow::ensure!(
        entries.next().is_none(),
        "refusing to overwrite non-empty output directory: {}",
        out.display()
    );
    Ok(())
}
