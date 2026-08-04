use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Store {
    Apple,
    Google,
}

impl Store {
    pub fn label(self) -> &'static str {
        match self {
            Self::Apple => "Apple App Store",
            Self::Google => "Google Play",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Apple => "apple",
            Self::Google => "google",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Screenshot,
    FeatureGraphic,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Target {
    pub id: String,
    pub store: Store,
    pub device: String,
    pub locale: String,
    #[serde(default = "default_asset_kind")]
    pub kind: AssetKind,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default = "default_min_assets")]
    pub min_assets: usize,
    #[serde(default)]
    pub max_assets: Option<usize>,
    #[serde(default)]
    pub exact_assets: Option<usize>,
    #[serde(default)]
    pub allowed_sizes: Vec<Size>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Criterion {
    pub id: String,
    pub name: String,
    pub weight: f64,
    pub guide: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Lens {
    pub id: String,
    pub name: String,
    pub instruction: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Experiment {
    #[serde(default = "default_experiment_variable")]
    pub variable: String,
    #[serde(default = "default_primary_metric")]
    pub primary_metric: String,
    #[serde(default)]
    pub guardrails: Vec<String>,
    #[serde(default = "default_min_days")]
    pub min_days: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Generation {
    pub brand_name: String,
    #[serde(default)]
    pub tagline: String,
    pub source_target: String,
    pub frame_count: usize,
    #[serde(default = "default_generator_backend")]
    pub generator_backend: String,
    #[serde(default = "default_generator_model")]
    pub generator_model: String,
    #[serde(default)]
    pub style_direction: String,
    pub palette: Vec<String>,
    #[serde(default = "default_layouts")]
    pub allowed_layouts: Vec<String>,
    #[serde(default = "default_creative_families")]
    pub creative_families: Vec<String>,
    #[serde(default)]
    pub art_direction: ArtDirection,
    #[serde(default)]
    pub segments: Vec<GenerationSegment>,
    /// Exact, verified tokens that may appear in numeric, ranking, award, rating, or
    /// guarantee-like claims. Empty means those trust markers are blocked.
    #[serde(default)]
    pub verified_claim_tokens: Vec<String>,
    #[serde(default)]
    pub store_tone_profiles: StoreToneProfiles,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArtDirection {
    /// Ordered store-story roles. Empty uses hero → overview → detail → synthesis.
    #[serde(default)]
    pub story_roles: Vec<String>,
    #[serde(default = "default_compositions")]
    pub allowed_compositions: Vec<String>,
    #[serde(default = "default_decorations")]
    pub allowed_decorations: Vec<String>,
    /// Optional per-frame accents. Values remain constrained to #RRGGBB.
    #[serde(default)]
    pub frame_accents: Vec<String>,
    #[serde(default = "default_max_repeated_composition")]
    pub max_consecutive_same_composition: usize,
    #[serde(default = "default_min_unique_compositions")]
    pub min_unique_compositions: usize,
}

impl Default for ArtDirection {
    fn default() -> Self {
        Self {
            story_roles: Vec::new(),
            allowed_compositions: default_compositions(),
            allowed_decorations: default_decorations(),
            frame_accents: Vec::new(),
            max_consecutive_same_composition: default_max_repeated_composition(),
            min_unique_compositions: default_min_unique_compositions(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenerationSegment {
    pub id: String,
    pub audience: String,
    pub intent: String,
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StoreToneProfile {
    #[serde(default)]
    pub voice: String,
    #[serde(default)]
    pub visual_direction: String,
    #[serde(default)]
    pub avoid_phrases: Vec<String>,
}

impl Default for StoreToneProfile {
    fn default() -> Self {
        Self {
            voice: "Human-forward, evidence-backed, and copy-first with minimal decoration."
                .to_string(),
            visual_direction:
                "Keep rhythm in margins and copy hierarchy. Never create a disconnected one-off motif."
                    .to_string(),
            avoid_phrases: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StoreToneProfiles {
    #[serde(default = "default_apple_tone_profile")]
    pub apple: StoreToneProfile,
    #[serde(default = "default_google_tone_profile")]
    pub google: StoreToneProfile,
}

impl StoreToneProfiles {
    pub fn profile(&self, store: Store) -> &StoreToneProfile {
        match store {
            Store::Apple => &self.apple,
            Store::Google => &self.google,
        }
    }
}

impl Default for StoreToneProfiles {
    fn default() -> Self {
        Self {
            apple: default_apple_tone_profile(),
            google: default_google_tone_profile(),
        }
    }
}

impl Default for Experiment {
    fn default() -> Self {
        Self {
            variable: default_experiment_variable(),
            primary_metric: default_primary_metric(),
            guardrails: vec!["1-day retention".to_string()],
            min_days: default_min_days(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Spec {
    pub name: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub audience: String,
    #[serde(default)]
    pub product_truth: Vec<String>,
    #[serde(default)]
    pub prohibited_claims: Vec<String>,
    #[serde(default = "default_thumbnail_width")]
    pub thumbnail_width: u32,
    #[serde(default = "default_critic_backends")]
    pub critic_backends: Vec<String>,
    #[serde(default = "default_openrouter_model")]
    pub openrouter_critic_model: String,
    pub targets: Vec<Target>,
    pub criteria: Vec<Criterion>,
    pub lenses: Vec<Lens>,
    #[serde(default)]
    pub generation: Option<Generation>,
    #[serde(default)]
    pub experiment: Experiment,
}

fn default_asset_kind() -> AssetKind {
    AssetKind::Screenshot
}
fn default_true() -> bool {
    true
}
fn default_min_assets() -> usize {
    1
}
fn default_thumbnail_width() -> u32 {
    220
}
fn default_critic_backends() -> Vec<String> {
    vec!["claude".into(), "openrouter".into(), "claude".into()]
}
fn default_openrouter_model() -> String {
    "openai/gpt-4.1-mini".to_string()
}
fn default_generator_backend() -> String {
    "openrouter".to_string()
}
fn default_generator_model() -> String {
    "openai/gpt-4.1-mini".to_string()
}
fn default_layouts() -> Vec<String> {
    vec![
        "ui_dominant".to_string(),
        "device_bottom".to_string(),
        "device_center".to_string(),
        "ui_focus".to_string(),
    ]
}
fn default_creative_families() -> Vec<String> {
    vec![
        "product_led".to_string(),
        "outcome_led".to_string(),
        "trust_led".to_string(),
    ]
}
fn default_compositions() -> Vec<String> {
    vec![
        "editorial_hero".to_string(),
        "editorial_split".to_string(),
        "chapter_field".to_string(),
        "synthesis_dark".to_string(),
    ]
}
fn default_decorations() -> Vec<String> {
    vec![
        "spectrum".to_string(),
        "orbit".to_string(),
        "grid".to_string(),
        "signal".to_string(),
    ]
}
fn default_max_repeated_composition() -> usize {
    2
}
fn default_min_unique_compositions() -> usize {
    1
}
fn default_experiment_variable() -> String {
    "store creative set".to_string()
}
fn default_primary_metric() -> String {
    "first-time download conversion rate".to_string()
}
fn default_min_days() -> u32 {
    7
}
fn default_apple_tone_profile() -> StoreToneProfile {
    StoreToneProfile {
        voice: "Refined, premium, and restrained. Keep the tone editorial and confident without hype."
            .to_string(),
        visual_direction: "Prioritize spacious composition, subtle contrast, and one clear affordance per frame."
            .to_string(),
        avoid_phrases: vec![
            "최고".to_string(),
            "최강".to_string(),
            "혁신".to_string(),
            "완벽".to_string(),
            "완전".to_string(),
            "무조건".to_string(),
            "절대".to_string(),
        ],
    }
}
fn default_google_tone_profile() -> StoreToneProfile {
    StoreToneProfile {
        voice: "Practical, clear, and action-oriented in local language. Lead with what is immediately useful."
            .to_string(),
        visual_direction:
            "Keep copy scannable and benefit-first, with a stronger call-out hierarchy than premium-only storytelling."
                .to_string(),
        avoid_phrases: vec![
            "AI".to_string(),
            "인공지능".to_string(),
            "즉시".to_string(),
            "완벽".to_string(),
            "무조건".to_string(),
            "절대".to_string(),
        ],
    }
}

impl Spec {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read spec: {}", path.display()))?;
        let spec: Self = toml::from_str(&raw)
            .with_context(|| format!("failed to parse TOML spec: {}", path.display()))?;
        spec.validate()?;
        Ok(spec)
    }

    fn validate(&self) -> Result<()> {
        anyhow::ensure!(!self.targets.is_empty(), "targets must not be empty");
        anyhow::ensure!(!self.criteria.is_empty(), "criteria must not be empty");
        anyhow::ensure!(!self.lenses.is_empty(), "lenses must not be empty");
        anyhow::ensure!(self.thumbnail_width >= 80, "thumbnail_width must be >= 80");
        anyhow::ensure!(
            self.experiment.min_days >= 1,
            "experiment.min_days must be >= 1"
        );

        ensure_unique("target", self.targets.iter().map(|x| x.id.as_str()))?;
        ensure_unique("criterion", self.criteria.iter().map(|x| x.id.as_str()))?;
        ensure_unique("lens", self.lenses.iter().map(|x| x.id.as_str()))?;

        for target in &self.targets {
            anyhow::ensure!(
                target.min_assets > 0,
                "target {} min_assets must be > 0",
                target.id
            );
            if let Some(max) = target.max_assets {
                anyhow::ensure!(
                    max >= target.min_assets,
                    "target {} max_assets < min_assets",
                    target.id
                );
            }
            if let Some(exact) = target.exact_assets {
                anyhow::ensure!(exact > 0, "target {} exact_assets must be > 0", target.id);
            }
            anyhow::ensure!(
                target
                    .allowed_sizes
                    .iter()
                    .all(|s| s.width > 0 && s.height > 0),
                "target {} contains an invalid allowed size",
                target.id
            );
        }
        anyhow::ensure!(
            self.criteria.iter().all(|c| c.weight > 0.0),
            "criterion weights must all be > 0"
        );
        anyhow::ensure!(
            self.critic_backends
                .iter()
                .all(|b| b == "claude" || b == "openrouter"),
            "critic_backends supports only claude and openrouter"
        );
        if let Some(generation) = &self.generation {
            anyhow::ensure!(
                !generation.brand_name.trim().is_empty(),
                "generation.brand_name must not be empty"
            );
            anyhow::ensure!(
                self.target(&generation.source_target).is_some(),
                "generation.source_target must reference a target id"
            );
            anyhow::ensure!(
                generation.frame_count > 0,
                "generation.frame_count must be greater than zero"
            );
            anyhow::ensure!(
                generation.generator_backend == "claude"
                    || generation.generator_backend == "openrouter",
                "generation.generator_backend supports only claude and openrouter"
            );
            anyhow::ensure!(
                generation.palette.len() >= 4,
                "generation.palette must contain at least four colors"
            );
            anyhow::ensure!(
                generation.palette.iter().all(|color| is_hex_color(color)),
                "generation.palette colors must use #RRGGBB"
            );
            anyhow::ensure!(
                !generation.allowed_layouts.is_empty(),
                "generation.allowed_layouts must not be empty"
            );
            anyhow::ensure!(
                generation.allowed_layouts.iter().all(|layout| matches!(
                    layout.as_str(),
                    "device_bottom" | "device_center" | "ui_focus" | "ui_dominant"
                )),
                "generation.allowed_layouts contains an unsupported layout"
            );
            anyhow::ensure!(
                !generation.creative_families.is_empty(),
                "generation.creative_families must not be empty"
            );
            anyhow::ensure!(
                generation.creative_families.iter().all(|family| matches!(
                    family.as_str(),
                    "product_led" | "outcome_led" | "trust_led"
                )),
                "generation.creative_families contains an unsupported family"
            );
            ensure_unique(
                "creative family",
                generation.creative_families.iter().map(String::as_str),
            )?;
            let art_direction = &generation.art_direction;
            anyhow::ensure!(
                art_direction.story_roles.is_empty()
                    || art_direction.story_roles.len() == generation.frame_count,
                "generation.art_direction.story_roles must be empty or match frame_count"
            );
            anyhow::ensure!(
                art_direction.story_roles.iter().all(|role| matches!(
                    role.as_str(),
                    "hero" | "overview" | "detail" | "proof" | "synthesis"
                )),
                "generation.art_direction.story_roles contains an unsupported role"
            );
            anyhow::ensure!(
                !art_direction.allowed_compositions.is_empty()
                    && art_direction
                        .allowed_compositions
                        .iter()
                        .all(|composition| matches!(
                            composition.as_str(),
                            "editorial_hero"
                                | "editorial_split"
                                | "chapter_field"
                                | "synthesis_dark"
                        )),
                "generation.art_direction.allowed_compositions contains an unsupported composition"
            );
            anyhow::ensure!(
                !art_direction.allowed_decorations.is_empty()
                    && art_direction
                        .allowed_decorations
                        .iter()
                        .all(|decoration| matches!(
                            decoration.as_str(),
                            "none" | "spectrum" | "orbit" | "grid" | "signal"
                        )),
                "generation.art_direction.allowed_decorations contains an unsupported decoration"
            );
            anyhow::ensure!(
                art_direction
                    .frame_accents
                    .iter()
                    .all(|color| is_hex_color(color)),
                "generation.art_direction.frame_accents colors must use #RRGGBB"
            );
            anyhow::ensure!(
                art_direction.max_consecutive_same_composition > 0,
                "generation.art_direction.max_consecutive_same_composition must be greater than zero"
            );
            anyhow::ensure!(
                art_direction.min_unique_compositions > 0
                    && art_direction.min_unique_compositions <= generation.frame_count
                    && art_direction.min_unique_compositions
                        <= art_direction.allowed_compositions.len(),
                "generation.art_direction.min_unique_compositions must fit frame_count and allowed_compositions"
            );
            ensure_unique(
                "art-direction composition",
                art_direction
                    .allowed_compositions
                    .iter()
                    .map(String::as_str),
            )?;
            ensure_unique(
                "art-direction decoration",
                art_direction.allowed_decorations.iter().map(String::as_str),
            )?;
            ensure_unique(
                "generation segment",
                generation
                    .segments
                    .iter()
                    .map(|segment| segment.id.as_str()),
            )?;
            anyhow::ensure!(
                generation.segments.iter().all(|segment| {
                    segment.id.chars().all(|character| {
                        character.is_ascii_alphanumeric() || character == '_' || character == '-'
                    })
                }),
                "generation segment ids support only ASCII letters, numbers, underscore, and hyphen"
            );
            anyhow::ensure!(
                generation.segments.iter().all(|segment| {
                    !segment.audience.trim().is_empty() && !segment.intent.trim().is_empty()
                }),
                "generation segments need non-empty audience and intent"
            );
            anyhow::ensure!(
                generation
                    .verified_claim_tokens
                    .iter()
                    .all(|token| !token.trim().is_empty()),
                "generation.verified_claim_tokens must not contain empty values"
            );
            anyhow::ensure!(
                !generation
                    .store_tone_profiles
                    .apple
                    .voice
                    .trim()
                    .is_empty(),
                "generation.store_tone_profiles.apple.voice must not be empty"
            );
            anyhow::ensure!(
                !generation
                    .store_tone_profiles
                    .apple
                    .visual_direction
                    .trim()
                    .is_empty(),
                "generation.store_tone_profiles.apple.visual_direction must not be empty"
            );
            anyhow::ensure!(
                !generation
                    .store_tone_profiles
                    .google
                    .voice
                    .trim()
                    .is_empty(),
                "generation.store_tone_profiles.google.voice must not be empty"
            );
            anyhow::ensure!(
                !generation
                    .store_tone_profiles
                    .google
                    .visual_direction
                    .trim()
                    .is_empty(),
                "generation.store_tone_profiles.google.visual_direction must not be empty"
            );
            anyhow::ensure!(
                generation
                    .store_tone_profiles
                    .apple
                    .avoid_phrases
                    .iter()
                    .all(|phrase| !phrase.trim().is_empty()),
                "generation.store_tone_profiles.apple.avoid_phrases must not contain empty values"
            );
            anyhow::ensure!(
                generation
                    .store_tone_profiles
                    .google
                    .avoid_phrases
                    .iter()
                    .all(|phrase| !phrase.trim().is_empty()),
                "generation.store_tone_profiles.google.avoid_phrases must not contain empty values"
            );
        }
        Ok(())
    }

    pub fn target(&self, id: &str) -> Option<&Target> {
        self.targets.iter().find(|t| t.id == id)
    }

    pub fn generation_segment(&self, id: &str) -> Result<GenerationSegment> {
        let generation = self
            .generation
            .as_ref()
            .context("spec has no [generation] section")?;
        if let Some(segment) = generation.segments.iter().find(|segment| segment.id == id) {
            return Ok(segment.clone());
        }
        if id == "default" {
            return Ok(GenerationSegment {
                id: "default".to_string(),
                audience: self.audience.clone(),
                intent: "general store discovery".to_string(),
                keywords: Vec::new(),
            });
        }
        anyhow::bail!("unknown generation segment: {id}")
    }
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn ensure_unique<'a>(label: &str, ids: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = HashSet::new();
    for id in ids {
        anyhow::ensure!(!id.trim().is_empty(), "{label} id must not be empty");
        anyhow::ensure!(seen.insert(id), "duplicate {label} id: {id}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_spec_parses_and_validates() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("specs/example.toml");
        let spec = Spec::load(&path).unwrap();
        assert_eq!(spec.targets.len(), 5);
        assert_eq!(spec.criteria.len(), 8);
        assert_eq!(spec.lenses.len(), 6);
        assert_eq!(spec.generation_segment("new_user").unwrap().id, "new_user");
    }
}
