mod contact_sheet;
mod critique;
mod discover;
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
    about = "Review app-store creative sets without confusing model consensus for market truth"
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
    }
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
