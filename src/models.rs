use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Note,
    Warn,
    Block,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Note => "NOTE",
            Self::Warn => "WARN",
            Self::Block => "BLOCK",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PolicyIssue {
    pub target_id: String,
    pub file: Option<String>,
    pub code: String,
    pub severity: Severity,
    pub evidence: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssetInfo {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub has_transparency: bool,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TargetAssets {
    pub target_id: String,
    pub assets: Vec<AssetInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Candidate {
    pub id: String,
    pub targets: Vec<TargetAssets>,
    pub policy_issues: Vec<PolicyIssue>,
    pub hard_pass: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CriterionScore {
    pub criterion_id: String,
    #[serde(deserialize_with = "deserialize_f64_from_number_or_string")]
    pub score: f64,
    pub evidence: String,
    pub why_not_higher: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VisualFinding {
    pub category: String,
    pub target_id: String,
    #[serde(deserialize_with = "deserialize_string_from_scalar")]
    pub frame: String,
    pub severity: Severity,
    pub evidence: String,
    pub suggested_fix: String,
}

fn deserialize_f64_from_number_or_string<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumberOrString {
        Number(f64),
        String(String),
    }

    match NumberOrString::deserialize(deserializer)? {
        NumberOrString::Number(value) => Ok(value),
        NumberOrString::String(value) => value
            .parse::<f64>()
            .map_err(|_| D::Error::custom(format!("expected a number, got {value:?}"))),
    }
}

fn deserialize_string_from_scalar<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        String(String),
        Integer(i64),
        Unsigned(u64),
        Number(f64),
    }

    Ok(match StringOrNumber::deserialize(deserializer)? {
        StringOrNumber::String(value) => value,
        StringOrNumber::Integer(value) => value.to_string(),
        StringOrNumber::Unsigned(value) => value.to_string(),
        StringOrNumber::Number(value) => value.to_string(),
    })
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CandidateCritique {
    pub candidate_id: String,
    pub first_glance: String,
    pub sequence_read: String,
    pub strongest_point: String,
    pub biggest_risk: String,
    pub criteria: Vec<CriterionScore>,
    #[serde(default)]
    pub findings: Vec<VisualFinding>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CritiqueRound {
    pub critic_index: usize,
    pub provider: String,
    pub lens_id: String,
    pub items: Vec<CandidateCritique>,
    pub ranking: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CorroboratedRisk {
    pub candidate_id: String,
    pub category: String,
    pub target_id: String,
    pub frame: String,
    pub reviewers: usize,
    pub max_severity: Severity,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QuantResult {
    pub borda: Vec<(String, u32)>,
    pub winner: Option<String>,
    pub criterion_means: BTreeMap<String, BTreeMap<String, f64>>,
    pub provider_diversity_note: String,
    pub unanimous_warning: Option<String>,
    pub minority_opinions: Vec<String>,
    pub corroborated_risks: Vec<CorroboratedRisk>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PriorObservation {
    pub candidate_id: String,
    pub category: String,
    pub target_id: String,
    pub frame: String,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct State {
    pub round: usize,
    pub spec_name: String,
    pub generated_at_note: String,
    pub blind_map: BTreeMap<String, String>,
    pub candidates: Vec<Candidate>,
    pub discourse: Vec<CritiqueRound>,
    pub quant: QuantResult,
    #[serde(default)]
    pub prior_observations: Vec<PriorObservation>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critic_scalars_accept_model_number_variants() {
        let score: CriterionScore = serde_json::from_value(serde_json::json!({
            "criterion_id": "hierarchy",
            "score": "4.5",
            "evidence": "visible",
            "why_not_higher": "small copy"
        }))
        .unwrap();
        let finding: VisualFinding = serde_json::from_value(serde_json::json!({
            "category": "hierarchy",
            "target_id": "phone",
            "frame": 3,
            "severity": "warn",
            "evidence": "visible",
            "suggested_fix": "enlarge"
        }))
        .unwrap();

        assert_eq!(score.score, 4.5);
        assert_eq!(finding.frame, "3");
    }
}
