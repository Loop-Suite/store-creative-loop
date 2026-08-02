use crate::llm::Llm;
use crate::models::{CandidateCritique, CritiqueRound};
use crate::spec::{Lens, Spec};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

const SYSTEM: &str = "You are one independent, blind reviewer of app-store creative sets. You do not know who made any candidate and you cannot see any other reviewer's opinion. Inspect the supplied images. Separate visible evidence from inference. Never claim conversion uplift or market validation. Return only the requested JSON.";

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
    let prompt = format!(
        "# Product\nContext: {}\nAudience: {}\nProduct truths: {}\nProhibited claims: {}\n\n\
         # Your review lens\n{} — {}\n\n\
         # Targets\n{}\n\n\
         # Candidate catalog\n{}\nEach file is a contact sheet. Frames read left-to-right, then top-to-bottom. Judge the whole multi-device set and identify target/frame-specific evidence.\n\n\
         # Scored criteria\n{}\n\n\
         Review every candidate. Use scores from 1 to 5. `findings.category` should use a stable short token such as hierarchy, copy_truth, sequence, accessibility, localization, device_fit, or policy. `frame` is a filename or 1-based position such as `2`; use `all` only when genuinely set-wide. Ranking must contain every candidate once, best offline recommendation first, with no ties.\n\n\
         Return JSON only:\n{{\"items\":[{{\"candidate_id\":\"candidate_01\",\"first_glance\":\"...\",\"sequence_read\":\"...\",\"strongest_point\":\"...\",\"biggest_risk\":\"...\",\"criteria\":[{{\"criterion_id\":\"...\",\"score\":1.0,\"evidence\":\"...\",\"why_not_higher\":\"...\"}}],\"findings\":[{{\"category\":\"hierarchy\",\"target_id\":\"...\",\"frame\":\"1\",\"severity\":\"warn\",\"evidence\":\"...\",\"suggested_fix\":\"...\"}}]}}],\"ranking\":[\"candidate_01\"]}}",
        spec.context,
        spec.audience,
        list_or_none(&spec.product_truth),
        list_or_none(&spec.prohibited_claims),
        lens.name,
        lens.instruction,
        targets,
        catalog.join("\n"),
        criteria
    );
    let raw: RawResponse = llm
        .json_with_images(&prompt, SYSTEM, &images)
        .with_context(|| format!("critic {critic_index} ({}) failed", llm.provider_label))?;
    validate_response(spec, &raw, blind_ids, critic_index)?;
    Ok(CritiqueRound {
        critic_index,
        provider: llm.provider_label.into(),
        lens_id: lens.id.clone(),
        items: raw.items,
        ranking: raw.ranking,
    })
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
