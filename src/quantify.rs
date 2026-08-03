use crate::models::{
    Candidate, CorroboratedRisk, CorroboratedSeriesRisk, CritiqueRound, QuantResult, Severity,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub fn quantify(candidates: &[Candidate], rounds: &[CritiqueRound]) -> QuantResult {
    let eligible = candidates
        .iter()
        .filter(|candidate| candidate.hard_pass)
        .map(|candidate| candidate.id.clone())
        .collect::<BTreeSet<_>>();
    let mut scores = eligible
        .iter()
        .map(|id| (id.clone(), 0_u32))
        .collect::<BTreeMap<_, _>>();
    let mut first_choices = Vec::new();

    for round in rounds {
        let ranking = round
            .ranking
            .iter()
            .filter(|id| eligible.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        if let Some(first) = ranking.first() {
            first_choices.push((round.critic_index, first.clone()));
        }
        let count = ranking.len() as u32;
        for (index, id) in ranking.iter().enumerate() {
            *scores.entry(id.clone()).or_default() += count.saturating_sub(index as u32);
        }
    }

    let mut borda = scores.into_iter().collect::<Vec<_>>();
    borda.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let corroborated_series_risks = corroborated_series_risks(rounds);
    let repair_required = corroborated_series_risks
        .iter()
        .map(|risk| risk.candidate_id.as_str())
        .collect::<BTreeSet<_>>();
    let winner = borda
        .iter()
        .find(|(id, _)| !repair_required.contains(id.as_str()))
        .map(|(id, _)| id.clone());
    let criterion_means = criterion_means(rounds);
    let provider_count = rounds
        .iter()
        .map(|round| round.provider.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let provider_diversity_note = if provider_count >= 2 {
        format!("{provider_count} independent provider families represented")
    } else {
        "single-provider panel: agreement may reflect correlated model behavior".into()
    };
    let unanimous_warning = if !first_choices.is_empty()
        && first_choices
            .iter()
            .all(|(_, choice)| choice == &first_choices[0].1)
    {
        Some(format!(
            "all critics ranked {} first; treat unanimity as a correlation warning, not proof",
            first_choices[0].1
        ))
    } else {
        None
    };
    let minority_opinions = winner
        .as_ref()
        .map(|winner| {
            first_choices
                .iter()
                .filter(|(_, choice)| choice != winner)
                .map(|(critic, choice)| {
                    format!("critic {critic} ranked {choice} first instead of {winner}")
                })
                .collect()
        })
        .unwrap_or_default();

    QuantResult {
        borda,
        winner,
        criterion_means,
        provider_diversity_note,
        unanimous_warning,
        minority_opinions,
        corroborated_risks: corroborated_risks(rounds),
        corroborated_series_risks,
    }
}

fn criterion_means(rounds: &[CritiqueRound]) -> BTreeMap<String, BTreeMap<String, f64>> {
    let mut values: HashMap<(String, String), Vec<f64>> = HashMap::new();
    for round in rounds {
        for item in &round.items {
            for score in &item.criteria {
                values
                    .entry((item.candidate_id.clone(), score.criterion_id.clone()))
                    .or_default()
                    .push(score.score);
            }
        }
    }
    let mut result: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for ((candidate, criterion), scores) in values {
        let mean = scores.iter().sum::<f64>() / scores.len() as f64;
        result.entry(candidate).or_default().insert(criterion, mean);
    }
    result
}

fn corroborated_risks(rounds: &[CritiqueRound]) -> Vec<CorroboratedRisk> {
    type Key = (String, String, String, String);
    let mut grouped: HashMap<Key, BTreeMap<usize, (Severity, String)>> = HashMap::new();
    for round in rounds {
        for item in &round.items {
            for finding in &item.findings {
                let key = (
                    item.candidate_id.clone(),
                    finding.category.trim().to_ascii_lowercase(),
                    finding.target_id.clone(),
                    finding.frame.clone(),
                );
                grouped.entry(key).or_default().insert(
                    round.critic_index,
                    (finding.severity, finding.evidence.clone()),
                );
            }
        }
    }
    let mut result = grouped
        .into_iter()
        .filter(|(_, reviewers)| reviewers.len() >= 2)
        .map(
            |((candidate_id, category, target_id, frame), reviewers)| CorroboratedRisk {
                candidate_id,
                category,
                target_id,
                frame,
                reviewers: reviewers.len(),
                max_severity: reviewers
                    .values()
                    .map(|(severity, _)| *severity)
                    .max()
                    .unwrap_or(Severity::Note),
                evidence: reviewers
                    .into_values()
                    .map(|(_, evidence)| evidence)
                    .collect(),
            },
        )
        .collect::<Vec<_>>();
    result.sort_by(|a, b| {
        b.max_severity
            .cmp(&a.max_severity)
            .then_with(|| b.reviewers.cmp(&a.reviewers))
            .then_with(|| a.candidate_id.cmp(&b.candidate_id))
    });
    result
}

#[derive(Debug, Clone)]
struct SeriesAuditRecord {
    severity: Severity,
    exceptions: Vec<String>,
    evidence: String,
    suggested_fix: String,
}

fn corroborated_series_risks(rounds: &[CritiqueRound]) -> Vec<CorroboratedSeriesRisk> {
    let mut grouped: HashMap<String, BTreeMap<usize, SeriesAuditRecord>> = HashMap::new();
    for round in rounds {
        for item in &round.items {
            let audit = &item.series_consistency;
            if audit.severity < Severity::Warn {
                continue;
            }
            grouped
                .entry(item.candidate_id.clone())
                .or_default()
                .insert(
                    round.critic_index,
                    SeriesAuditRecord {
                        severity: audit.severity,
                        exceptions: audit.exceptions.clone(),
                        evidence: audit.evidence.clone(),
                        suggested_fix: audit.suggested_fix.clone(),
                    },
                );
        }
    }
    let mut result = grouped
        .into_iter()
        .filter(|(_, reviewers)| reviewers.len() >= 2)
        .map(|(candidate_id, reviewers)| CorroboratedSeriesRisk {
            candidate_id,
            reviewers: reviewers.len(),
            max_severity: reviewers
                .values()
                .map(|record| record.severity)
                .max()
                .unwrap_or(Severity::Warn),
            exceptions: reviewers
                .values()
                .flat_map(|record| record.exceptions.iter().cloned())
                .collect(),
            evidence: reviewers
                .values()
                .map(|record| record.evidence.clone())
                .collect(),
            suggested_fixes: reviewers
                .into_values()
                .map(|record| record.suggested_fix)
                .collect(),
        })
        .collect::<Vec<_>>();
    result.sort_by(|a, b| {
        b.max_severity
            .cmp(&a.max_severity)
            .then_with(|| b.reviewers.cmp(&a.reviewers))
            .then_with(|| a.candidate_id.cmp(&b.candidate_id))
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CandidateCritique, CriterionScore, SeriesConsistencyAudit, TargetAssets, VisualFinding,
    };

    fn candidate(id: &str, hard_pass: bool) -> Candidate {
        Candidate {
            id: id.into(),
            targets: Vec::<TargetAssets>::new(),
            policy_issues: vec![],
            hard_pass,
        }
    }

    fn item(candidate_id: &str, evidence: &str) -> CandidateCritique {
        CandidateCritique {
            candidate_id: candidate_id.into(),
            first_glance: "clear".into(),
            sequence_read: "ordered".into(),
            strongest_point: "focus".into(),
            biggest_risk: "copy".into(),
            criteria: vec![CriterionScore {
                criterion_id: "clarity".into(),
                score: 4.0,
                evidence: "visible".into(),
                why_not_higher: "dense".into(),
            }],
            series_consistency: SeriesConsistencyAudit {
                severity: Severity::Warn,
                recurring_elements: vec!["brand lockup".into()],
                exceptions: vec!["oversized 01 appears only on frame 2".into()],
                evidence: "frame 2 alone adds a large decorative numeral".into(),
                suggested_fix: "remove the numeral or use one consistent counter system".into(),
            },
            findings: vec![VisualFinding {
                category: "hierarchy".into(),
                target_id: "phone".into(),
                frame: "1".into(),
                severity: Severity::Warn,
                evidence: evidence.into(),
                suggested_fix: "reduce copy".into(),
            }],
        }
    }

    #[test]
    fn blocked_candidates_cannot_win_and_two_critics_corroborate_a_risk() {
        let candidates = vec![
            candidate("a", true),
            candidate("b", true),
            candidate("blocked", false),
        ];
        let rounds = vec![
            CritiqueRound {
                critic_index: 1,
                provider: "claude".into(),
                lens_id: "user".into(),
                items: vec![item("a", "headline competes with UI")],
                ranking: vec!["blocked".into(), "a".into(), "b".into()],
            },
            CritiqueRound {
                critic_index: 2,
                provider: "openrouter".into(),
                lens_id: "designer".into(),
                items: vec![item("a", "too many equal focal points")],
                ranking: vec!["blocked".into(), "b".into(), "a".into()],
            },
        ];
        let result = quantify(&candidates, &rounds);
        assert!(result.borda.iter().all(|(id, _)| id != "blocked"));
        assert_eq!(result.winner.as_deref(), Some("b"));
        assert_eq!(result.corroborated_risks.len(), 1);
        assert_eq!(result.corroborated_risks[0].reviewers, 2);
        assert_eq!(result.corroborated_series_risks.len(), 1);
        assert_eq!(result.corroborated_series_risks[0].reviewers, 2);
        assert!(result.corroborated_series_risks[0].exceptions[0].contains("oversized 01"));
        assert!(result.corroborated_series_risks[0].evidence[0].contains("frame 2"));
        assert!(result.provider_diversity_note.starts_with('2'));
    }
}
