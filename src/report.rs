use crate::discover::issue_counts;
use crate::models::{Severity, State};
use crate::spec::Spec;
use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::path::Path;

pub fn write_reports(out: &Path, spec: &Spec, state: &State) -> Result<()> {
    std::fs::write(out.join("report.md"), render_report(spec, state))
        .with_context(|| format!("failed to write {}/report.md", out.display()))?;
    std::fs::write(out.join("experiment.md"), render_experiment(spec, state))
        .with_context(|| format!("failed to write {}/experiment.md", out.display()))?;
    Ok(())
}

pub fn render_report(spec: &Spec, state: &State) -> String {
    let mut report = String::new();
    writeln!(report, "# {} — review round {}\n", spec.name, state.round).unwrap();
    writeln!(
        report,
        "> **Verdict boundary:** this is an offline, model-assisted recommendation. It is not market validation and does not establish conversion uplift. Run the experiment handoff before making a causal claim.\n"
    )
    .unwrap();

    writeln!(report, "## Deterministic policy gates\n").unwrap();
    writeln!(report, "| Candidate | Result | BLOCK | WARN | NOTE |").unwrap();
    writeln!(report, "|---|---:|---:|---:|---:|").unwrap();
    for candidate in &state.candidates {
        let counts = issue_counts(candidate);
        writeln!(
            report,
            "| {} | {} | {} | {} | {} |",
            candidate.id,
            if candidate.hard_pass {
                "PASS"
            } else {
                "BLOCKED"
            },
            counts.get(&Severity::Block).copied().unwrap_or(0),
            counts.get(&Severity::Warn).copied().unwrap_or(0),
            counts.get(&Severity::Note).copied().unwrap_or(0)
        )
        .unwrap();
    }
    for candidate in &state.candidates {
        if candidate.policy_issues.is_empty() {
            continue;
        }
        writeln!(report, "\n### {} issues\n", candidate.id).unwrap();
        for issue in &candidate.policy_issues {
            writeln!(
                report,
                "- **{} `{}`** `{}`{} — {}",
                issue.severity.label(),
                issue.code,
                issue.target_id,
                issue
                    .file
                    .as_ref()
                    .map(|file| format!(" / `{file}`"))
                    .unwrap_or_default(),
                issue.evidence
            )
            .unwrap();
        }
    }

    writeln!(report, "\n## Offline recommendation\n").unwrap();
    match &state.quant.winner {
        Some(winner) => writeln!(report, "Recommended candidate: **{}**\n", winner).unwrap(),
        None => writeln!(
            report,
            "No eligible candidate. Resolve deterministic policy blocks or corroborated series-consistency repairs before handoff.\n"
        )
        .unwrap(),
    }
    writeln!(report, "| Rank | Candidate | Borda points |").unwrap();
    writeln!(report, "|---:|---|---:|").unwrap();
    for (index, (candidate, score)) in state.quant.borda.iter().enumerate() {
        writeln!(report, "| {} | {} | {} |", index + 1, candidate, score).unwrap();
    }
    writeln!(
        report,
        "\n- Panel note: {}",
        state.quant.provider_diversity_note
    )
    .unwrap();
    if let Some(warning) = &state.quant.unanimous_warning {
        writeln!(report, "- Unanimity warning: {warning}").unwrap();
    }
    for opinion in &state.quant.minority_opinions {
        writeln!(report, "- Minority opinion: {opinion}").unwrap();
    }

    writeln!(report, "\n## Criterion means (1–5)\n").unwrap();
    for (candidate, criteria) in &state.quant.criterion_means {
        writeln!(report, "### {candidate}\n").unwrap();
        for (criterion, mean) in criteria {
            writeln!(report, "- `{criterion}`: {mean:.2}").unwrap();
        }
        writeln!(report).unwrap();
    }

    writeln!(report, "## Corroborated series-consistency risks\n").unwrap();
    writeln!(
        report,
        "This is a model-assisted visual-grammar gate, separate from deterministic file and platform policy checks.\n"
    )
    .unwrap();
    if state.quant.corroborated_series_risks.is_empty() {
        writeln!(
            report,
            "No unexplained one-off overlay was independently reported by at least two critics.\n"
        )
        .unwrap();
    }
    for risk in &state.quant.corroborated_series_risks {
        writeln!(
            report,
            "- **{}** — repair required before final handoff; {} reviewers; max severity {}",
            risk.candidate_id,
            risk.reviewers,
            risk.max_severity.label()
        )
        .unwrap();
        for exception in &risk.exceptions {
            writeln!(report, "  - Exception: {exception}").unwrap();
        }
        for evidence in &risk.evidence {
            writeln!(report, "  - Evidence: {evidence}").unwrap();
        }
        for suggested_fix in &risk.suggested_fixes {
            writeln!(report, "  - Repair: {suggested_fix}").unwrap();
        }
    }

    writeln!(report, "## Corroborated risks\n").unwrap();
    if state.quant.corroborated_risks.is_empty() {
        writeln!(
            report,
            "No finding was independently reported by at least two critics.\n"
        )
        .unwrap();
    }
    for risk in &state.quant.corroborated_risks {
        writeln!(
            report,
            "- **{} / {}** — `{}` frame `{}`; {} reviewers; max severity {}",
            risk.candidate_id,
            risk.category,
            risk.target_id,
            risk.frame,
            risk.reviewers,
            risk.max_severity.label()
        )
        .unwrap();
        for evidence in &risk.evidence {
            writeln!(report, "  - {evidence}").unwrap();
        }
    }

    if !state.prior_observations.is_empty() {
        writeln!(report, "\n## Prior-round observations\n").unwrap();
        writeln!(report, "`NOT_REOBSERVED` deliberately does not mean fixed; it only means this panel did not independently reproduce the finding.\n").unwrap();
        for observation in &state.prior_observations {
            writeln!(
                report,
                "- **{}** — {} / {} / {} / frame {}",
                observation.status,
                observation.candidate_id,
                observation.category,
                observation.target_id,
                observation.frame
            )
            .unwrap();
        }
    }

    writeln!(report, "\n## Independent critic records\n").unwrap();
    for round in &state.discourse {
        writeln!(
            report,
            "### Critic {} — provider `{}`, lens `{}`\n\nRanking: {}\n",
            round.critic_index,
            round.provider,
            round.lens_id,
            round.ranking.join(" → ")
        )
        .unwrap();
        for item in &round.items {
            writeln!(
                report,
                "- **{}** — first glance: {} Strongest point: {} Biggest risk: {} Series grammar {}: {}",
                item.candidate_id,
                item.first_glance,
                item.strongest_point,
                item.biggest_risk,
                item.series_consistency.severity.label(),
                item.series_consistency.evidence
            )
            .unwrap();
        }
    }
    writeln!(
        report,
        "\nSee [experiment.md](experiment.md) for the market-validation handoff."
    )
    .unwrap();
    report
}

pub fn render_experiment(spec: &Spec, state: &State) -> String {
    let treatment = state
        .quant
        .winner
        .as_deref()
        .unwrap_or("no eligible treatment yet");
    let mut report = format!(
        "# Store experiment handoff\n\n> This is a pre-registration template, not evidence of uplift. Change one declared variable at a time.\n\n- **Hypothesis:** `{treatment}` will improve `{}` for the stated audience.\n- **Control:** current production store creative\n- **Treatment:** `{treatment}`\n- **Declared variable:** {}\n- **Primary metric:** {}\n- **Minimum run window:** {} full days, then continue until the platform's sample requirements are satisfied\n- **Guardrails:** {}\n\n",
        spec.experiment.primary_metric,
        spec.experiment.variable,
        spec.experiment.primary_metric,
        spec.experiment.min_days,
        if spec.experiment.guardrails.is_empty() { "none supplied".into() } else { spec.experiment.guardrails.join(", ") }
    );
    report.push_str(
        "## Platform execution\n\n- Apple: use Product Page Optimization where eligible; preserve the original as control and record treatment allocation.\n- Google Play: use Store Listing Experiments; localize the treatment to the same audience and do not mix copy, screenshots, icon, and pricing in one causal claim.\n- Save exposure dates, allocation, locale, device mix, point estimate, confidence interval, and guardrail movement back into the next round's notes.\n\n## Decision rule\n\nDecide the rule before reading results. Ship only when the primary metric clears the agreed practical threshold without a material guardrail regression. Otherwise retain the control or run a newly registered iteration.\n",
    );
    report
}
