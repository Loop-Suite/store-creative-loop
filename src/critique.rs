use crate::llm::Llm;
use crate::models::{CandidateCritique, CritiqueRound};
use crate::spec::{Lens, Spec};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

const SYSTEM: &str = "You are one independent, blind reviewer of app-store creative sets. You do not know who made any candidate and you cannot see any other reviewer's opinion. Inspect the supplied images. Separate visible evidence from inference. Never claim conversion uplift or market validation. Return only the requested JSON.";
const SCHEMA_ATTEMPTS: usize = 3;

#[derive(Debug, Deserialize)]
struct RawResponse {
    items: Vec<CandidateCritique>,
    ranking: Vec<String>,
}

pub fn run_one(
    llm: &Llm,
    spec: &Spec,
    lens: &Lens,
    blind_ids: &[String],
    sheets: &BTreeMap<String, Vec<PathBuf>>,
    critic_index: usize,
) -> Result<CritiqueRound> {
    let mut shifted = blind_ids.to_vec();
    if !shifted.is_empty() {
        let len = shifted.len();
        shifted.rotate_left(critic_index % len);
    }
    let mut images = Vec::new();
    let mut catalog = Vec::new();
    for id in &shifted {
        let paths = sheets.get(id).cloned().unwrap_or_default();
        catalog.push(format!(
            "- {id}: {}",
            paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        images.extend(paths);
    }
    let criteria = spec
        .criteria
        .iter()
        .map(|criterion| {
            format!(
                "- {} (1–5, weight {}): {}",
                criterion.id, criterion.weight, criterion.guide
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let targets = spec
        .targets
        .iter()
        .map(|target| {
            format!(
                "- {}: {}, {}, locale {}, {:?}",
                target.id,
                target.store.label(),
                target.device,
                target.locale,
                target.kind
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let response_template = response_template(spec, &shifted)?;
    let prompt = format!(
        "# Product\nContext: {}\nAudience: {}\nProduct truths: {}\nProhibited claims: {}\n\n\
         # Your review lens\n{} — {}\n\n\
         # Targets\n{}\n\n\
         # Candidate catalog\n{}\nEach file is a contact sheet. Frames read left-to-right, then top-to-bottom. Judge the whole multi-device set and identify target/frame-specific evidence.\n\n\
         # Scored criteria\n{}\n\n\
         Review every candidate. Use scores from 1 to 5. Be concise: keep each summary, evidence, limitation, and fix to one sentence of at most 25 words; return only the 1–3 most decision-relevant findings per candidate; keep the complete JSON response below 4,000 tokens. `findings.category` should use a stable short token such as hierarchy, copy_truth, sequence, accessibility, localization, device_fit, or policy. Every finding must contain `category`, `target_id`, `frame`, `severity`, `evidence`, and `suggested_fix`; `target_id` must be one of the listed target ids, `severity` must be `note`, `warn`, or `block`, and `frame` is a filename or 1-based position such as `2` (`all` only when genuinely set-wide). Ranking must contain every candidate once, best offline recommendation first, with no ties.\n\n\
         The response must contain exactly these candidate ids: {}. Every candidate needs exactly one item, every item needs all listed criterion ids exactly once, and ranking must be a permutation of the same candidate ids.\n\n\
         Return JSON only. Use this complete template, replace every placeholder, and reorder `ranking` based on your judgment:\n{}",
        spec.context,
        spec.audience,
        list_or_none(&spec.product_truth),
        list_or_none(&spec.prohibited_claims),
        lens.name,
        lens.instruction,
        targets,
        catalog.join("\n"),
        criteria,
        shifted.join(", "),
        response_template
    );
    let raw: RawResponse = retry_with_validation(
        &prompt,
        SCHEMA_ATTEMPTS,
        |request_prompt| llm.json_with_images(request_prompt, SYSTEM, &images),
        |response| validate_response(spec, response, blind_ids, critic_index),
    )
    .with_context(|| format!("critic {critic_index} ({}) failed", llm.provider_label))?;
    Ok(CritiqueRound {
        critic_index,
        provider: llm.provider_label.into(),
        lens_id: lens.id.clone(),
        items: raw.items,
        ranking: raw.ranking,
    })
}

fn response_template(spec: &Spec, ids: &[String]) -> Result<String> {
    let example_target = spec
        .targets
        .first()
        .context("cannot build response template without a target")?
        .id
        .clone();
    let criteria = spec
        .criteria
        .iter()
        .map(|criterion| {
            serde_json::json!({
                "criterion_id": criterion.id,
                "score": 1.0,
                "evidence": "replace with visible evidence",
                "why_not_higher": "replace with the limiting factor"
            })
        })
        .collect::<Vec<_>>();
    let items = ids
        .iter()
        .map(|id| {
            serde_json::json!({
                "candidate_id": id,
                "first_glance": "replace",
                "sequence_read": "replace",
                "strongest_point": "replace",
                "biggest_risk": "replace",
                "criteria": criteria,
                "findings": [{
                    "category": "hierarchy",
                    "target_id": example_target,
                    "frame": "1",
                    "severity": "warn",
                    "evidence": "replace with visible evidence",
                    "suggested_fix": "replace with a concrete fix"
                }]
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&serde_json::json!({"items": items, "ranking": ids}))
        .context("failed to build response template")
}

fn retry_with_validation<T, Request, Validate>(
    base_prompt: &str,
    attempts: usize,
    mut request: Request,
    mut validate: Validate,
) -> Result<T>
where
    Request: FnMut(&str) -> Result<T>,
    Validate: FnMut(&T) -> Result<()>,
{
    anyhow::ensure!(attempts > 0, "schema attempts must be greater than zero");
    let mut prompt = base_prompt.to_string();
    let mut last_error = None;
    for attempt in 1..=attempts {
        let result = request(&prompt).and_then(|value| {
            validate(&value)?;
            Ok(value)
        });
        match result {
            Ok(value) => return Ok(value),
            Err(error) => {
                let detail = format!("{error:#}");
                if attempt < attempts {
                    eprintln!(
                        "warning: critic response rejected on schema attempt {attempt}/{attempts}: {detail}"
                    );
                    prompt = format!(
                        "{base_prompt}\n\n# Correction required\nYour previous response was rejected: {detail}. Return a completely new JSON response that satisfies every field, coverage, enum, and range constraint. Do not omit, rename, or duplicate any candidate or criterion id. Every finding must include category, target_id, frame, severity, evidence, and suggested_fix. Keep every text value to one short sentence, include at most three findings per candidate, and keep the entire response below 4,000 tokens."
                    );
                }
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("critic response validation failed")))
}

fn validate_response(
    spec: &Spec,
    raw: &RawResponse,
    ids: &[String],
    critic_index: usize,
) -> Result<()> {
    let expected = ids.iter().cloned().collect::<HashSet<_>>();
    let ranking = raw.ranking.iter().cloned().collect::<HashSet<_>>();
    anyhow::ensure!(
        ranking == expected && raw.ranking.len() == ids.len(),
        "critic {critic_index} ranking must contain every candidate exactly once"
    );
    let item_ids = raw
        .items
        .iter()
        .map(|item| item.candidate_id.clone())
        .collect::<HashSet<_>>();
    anyhow::ensure!(
        item_ids == expected && raw.items.len() == ids.len(),
        "critic {critic_index} item coverage mismatch"
    );
    let criterion_ids = spec
        .criteria
        .iter()
        .map(|criterion| criterion.id.as_str())
        .collect::<HashSet<_>>();
    for item in &raw.items {
        let returned = item
            .criteria
            .iter()
            .map(|score| score.criterion_id.as_str())
            .collect::<HashSet<_>>();
        anyhow::ensure!(
            returned == criterion_ids,
            "critic {critic_index} criterion coverage mismatch for {}",
            item.candidate_id
        );
        anyhow::ensure!(
            item.criteria
                .iter()
                .all(|score| (1.0..=5.0).contains(&score.score)),
            "critic {critic_index} returned a score outside 1–5"
        );
        anyhow::ensure!(
            item.findings
                .iter()
                .all(|finding| spec.target(&finding.target_id).is_some()),
            "critic {critic_index} returned an unknown target id"
        );
    }
    Ok(())
}

pub fn unblind(rounds: &mut [CritiqueRound], blind_map: &BTreeMap<String, String>) -> Result<()> {
    for round in rounds {
        for id in &mut round.ranking {
            *id = blind_map
                .get(id)
                .with_context(|| format!("unknown blind id: {id}"))?
                .clone();
        }
        for item in &mut round.items {
            item.candidate_id = blind_map
                .get(&item.candidate_id)
                .with_context(|| format!("unknown blind id: {}", item.candidate_id))?
                .clone();
        }
    }
    Ok(())
}

fn list_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "none supplied".into()
    } else {
        values.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_schema_failure_is_retried_with_correction_context() {
        let mut calls = 0;
        let value = retry_with_validation(
            "base prompt",
            3,
            |prompt| {
                calls += 1;
                if calls == 2 {
                    assert!(prompt.contains("Correction required"));
                    assert!(prompt.contains("expected 2"));
                }
                Ok(calls)
            },
            |value| {
                anyhow::ensure!(*value == 2, "expected 2");
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(value, 2);
        assert_eq!(calls, 2);
    }
}
