use crate::llm::Llm;
use crate::spec::{AssetKind, Generation, GenerationSegment, Spec, Store, Target};
use ab_glyph::{FontArc, PxScale};
use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::{Rgba, RgbaImage};
use imageproc::drawing::{
    draw_filled_circle_mut, draw_filled_rect_mut, draw_hollow_circle_mut, draw_line_segment_mut,
    draw_polygon_mut, draw_text_mut, text_size,
};
use imageproc::point::Point;
use imageproc::rect::Rect;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

const PLAN_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    UiDominant,
    DeviceBottom,
    DeviceCenter,
    UiFocus,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryRole {
    #[default]
    Legacy,
    Hero,
    Overview,
    Detail,
    Proof,
    Synthesis,
}

impl StoryRole {
    fn id(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Hero => "hero",
            Self::Overview => "overview",
            Self::Detail => "detail",
            Self::Proof => "proof",
            Self::Synthesis => "synthesis",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Composition {
    #[default]
    Legacy,
    EditorialHero,
    EditorialSplit,
    ChapterField,
    SynthesisDark,
}

impl Composition {
    fn id(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::EditorialHero => "editorial_hero",
            Self::EditorialSplit => "editorial_split",
            Self::ChapterField => "chapter_field",
            Self::SynthesisDark => "synthesis_dark",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decoration {
    #[default]
    None,
    Spectrum,
    Orbit,
    Grid,
    Signal,
}

impl Decoration {
    fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Spectrum => "spectrum",
            Self::Orbit => "orbit",
            Self::Grid => "grid",
            Self::Signal => "signal",
        }
    }
}

impl Layout {
    fn id(self) -> &'static str {
        match self {
            Self::UiDominant => "ui_dominant",
            Self::DeviceBottom => "device_bottom",
            Self::DeviceCenter => "device_center",
            Self::UiFocus => "ui_focus",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum CreativeFamily {
    #[default]
    #[serde(rename = "product_led")]
    Product,
    #[serde(rename = "outcome_led")]
    Outcome,
    #[serde(rename = "trust_led")]
    Trust,
}

impl CreativeFamily {
    pub fn id(self) -> &'static str {
        match self {
            Self::Product => "product_led",
            Self::Outcome => "outcome_led",
            Self::Trust => "trust_led",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PalettePlan {
    pub background_start: String,
    pub background_end: String,
    pub accent: String,
    pub text: String,
    pub muted: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FramePlan {
    pub index: usize,
    pub badge: String,
    pub headline: String,
    pub body: String,
    #[serde(default)]
    pub chips: Vec<String>,
    pub layout: Layout,
    #[serde(default)]
    pub role: StoryRole,
    #[serde(default)]
    pub composition: Composition,
    #[serde(default)]
    pub decoration: Decoration,
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub footer: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FeaturePlan {
    pub headline: String,
    pub body: String,
    #[serde(default)]
    pub chips: Vec<String>,
    pub source_indices: Vec<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CreativePlan {
    pub id: String,
    #[serde(default)]
    pub family: CreativeFamily,
    #[serde(default)]
    pub hypothesis_id: String,
    #[serde(default)]
    pub hypothesis: String,
    pub concept: String,
    pub palette: PalettePlan,
    pub frames: Vec<FramePlan>,
    pub feature: FeaturePlan,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PlanResponse {
    plans: Vec<CreativePlan>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GenerationManifest {
    pub round: usize,
    pub generator_provider: String,
    #[serde(default)]
    pub segment: Option<GenerationSegment>,
    pub raw_sources: Vec<String>,
    #[serde(default)]
    pub target_sources: BTreeMap<String, Vec<String>>,
    pub plans: Vec<CreativePlan>,
}

#[derive(Debug, Clone)]
pub struct RawSourceCatalog {
    primary_target: String,
    by_target: BTreeMap<String, Vec<PathBuf>>,
}

impl RawSourceCatalog {
    pub fn primary(&self) -> &[PathBuf] {
        self.by_target
            .get(&self.primary_target)
            .expect("primary raw source target must exist")
    }

    fn for_target(&self, target_id: &str) -> &[PathBuf] {
        self.by_target
            .get(target_id)
            .map(Vec::as_slice)
            .unwrap_or_else(|| self.primary())
    }
}

pub fn discover_raw_sources(
    raw_root: &Path,
    spec: &Spec,
    generation: &Generation,
) -> Result<RawSourceCatalog> {
    let primary = discover_source_dir(raw_root, &generation.source_target, generation.frame_count)?;
    let mut by_target = BTreeMap::from([(generation.source_target.clone(), primary)]);
    for target in spec
        .targets
        .iter()
        .filter(|target| target.kind == AssetKind::Screenshot)
    {
        if target.id == generation.source_target {
            continue;
        }
        let source_dir = raw_root.join(&target.id);
        if source_dir.is_dir() {
            by_target.insert(
                target.id.clone(),
                discover_source_dir(raw_root, &target.id, generation.frame_count)?,
            );
        }
    }
    Ok(RawSourceCatalog {
        primary_target: generation.source_target.clone(),
        by_target,
    })
}

fn discover_source_dir(
    raw_root: &Path,
    target_id: &str,
    frame_count: usize,
) -> Result<Vec<PathBuf>> {
    let source_dir = raw_root.join(target_id);
    let mut sources = std::fs::read_dir(&source_dir)
        .with_context(|| {
            format!(
                "failed to read raw source directory: {}",
                source_dir.display()
            )
        })?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .map(|entry| entry.path())
        .filter(|path| {
            matches!(
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .as_str(),
                "png" | "jpg" | "jpeg" | "webp"
            )
        })
        .collect::<Vec<_>>();
    sources.sort();
    anyhow::ensure!(
        sources.len() >= frame_count,
        "generation needs {} raw images in {}, found {}",
        frame_count,
        source_dir.display(),
        sources.len()
    );
    sources.truncate(frame_count);
    Ok(sources)
}

#[allow(clippy::too_many_arguments)]
pub fn generate_plans(
    llm: &Llm,
    spec: &Spec,
    generation: &Generation,
    sources: &[PathBuf],
    contact_sheet: &Path,
    variants: usize,
    round: usize,
    segment: &GenerationSegment,
    feedback: Option<&str>,
) -> Result<Vec<CreativePlan>> {
    let template = plan_template(generation, variants, round, segment)?;
    let mut store_tones = Vec::new();
    if spec.targets.iter().any(|target| target.store == Store::Apple) {
        let profile = &generation.store_tone_profiles.apple;
        store_tones.push(format!(
            "# Apple App Store\n- Voice: {}\n- Visual direction: {}\n- Avoid phrases: {}",
            profile.voice,
            profile.visual_direction,
            list_or_none(&profile.avoid_phrases),
        ));
    }
    if spec.targets.iter().any(|target| target.store == Store::Google) {
        let profile = &generation.store_tone_profiles.google;
        store_tones.push(format!(
            "# Google Play\n- Voice: {}\n- Visual direction: {}\n- Avoid phrases: {}",
            profile.voice,
            profile.visual_direction,
            list_or_none(&profile.avoid_phrases),
        ));
    }
    let store_tones = if store_tones.is_empty() {
        "No explicit store-specific tone profiles configured; keep one premium brand voice across targets."
            .to_string()
    } else {
        store_tones.join("\n\n")
    };
    let role_sequence = story_roles(generation)?
        .iter()
        .map(|role| role.id())
        .collect::<Vec<_>>()
        .join(" → ");
    let source_catalog = sources
        .iter()
        .enumerate()
        .map(|(index, path)| format!("- frame {}: {}", index + 1, path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "# Product\nContext: {}\nGeneral audience: {}\nProduct truths: {}\nProhibited claims: {}\n\n\
         # Store-adaptive creative direction\n{}\n\n\
         # Selected store segment\nSegment id: {}\nAudience: {}\nIntent: {}\nKeywords: {}\n\n\
         # Store-creative generation task\nCreate exactly {variants} production-ready creative plans for round {round}. Follow the exact creative family assigned to each template plan: `product_led` makes real UI dominant and demonstrates a concrete task; `outcome_led` leads with the user's desired outcome or emotion; `trust_led` emphasizes clarity, control, and visible evidence without inventing social proof. Each plan turns the same ordered raw app captures into a coherent screenshot story. The deterministic renderer—not you—will draw the final pixels.\n\n\
         Brand: {}\nTagline: {}\nStyle direction: {}\nAllowed palette values (use these exact strings only): {}\nAllowed layouts: {}\nStory roles: {}\nAllowed compositions: {}\nAllowed decorations: {}\nFrame accent options: {}\nRequired screenshot frames per plan: {}\n\n\
         # Ordered raw captures\n{}\nThe attached image is their contact sheet in the same left-to-right, top-to-bottom order. Do not invent product UI.\n\n\
         # Prior-round feedback\n{}\n\n\
         Write concise store copy, not captions that merely describe the pixels. Headline: at most 24 Korean characters or 42 Latin characters and at most two visual lines. Body: one short sentence. Use 0–3 short chips. Preserve every assigned story role, composition, decoration, accent, layout, and frame index from the template. Write `footer` as a short payoff or three-part reading axis that adds information without repeating the headline. The first frame must use `ui_dominant` when that layout is allowed, communicate one benefit, and avoid decorative clutter. Make the first three frames carry the product promise, evidence, and differentiation. Later frames should add non-redundant evidence. Visual rhythm comes from role-specific compositions and decorations, not from repeating one centered device template.\n\n\
         Keep copy premium and human-crafted: avoid one-dimensional promotional language, abstract claims without UI evidence, and every listed store-profile avoid phrase. Treat every non-product overlay as either a repeated set-wide system or a clearly semantic frame-specific motif. Never introduce an isolated decorative numeral, counter, badge, icon, divider, or card merely to fill space. If a position counter is used, it must form one complete sequence with the same placement, scale, and style on every screenshot; do not enlarge one frame number. A frame-specific motif must explain that frame's content without resembling product UI or unverified data. Variants may explore their assigned family, but must not change product truth. Feature art must work at 1024x500 and may use one or two valid source indices.\n\n\
         Never write rankings, awards, ratings, review counts, download counts, percentages, guarantees, or superlatives unless the exact supporting token appears in this verified allowlist: {}. An empty allowlist means all such trust markers are prohibited.\n\n\
         Return JSON only. Keep every id/index/field in this complete template, replace all placeholder copy, and preserve the exact number of plans and frames:\n{}",
        spec.context,
        spec.audience,
        list_or_none(&spec.product_truth),
        list_or_none(&spec.prohibited_claims),
        store_tones,
        segment.id,
        segment.audience,
        segment.intent,
        list_or_none(&segment.keywords),
        generation.brand_name,
        generation.tagline,
        generation.style_direction,
        generation.palette.join(", "),
        generation.allowed_layouts.join(", "),
        role_sequence,
        generation.art_direction.allowed_compositions.join(", "),
        generation.art_direction.allowed_decorations.join(", "),
        list_or_none(&generation.art_direction.frame_accents),
        generation.frame_count,
        source_catalog,
        feedback.unwrap_or("No prior round. Explore distinct evidence-backed directions."),
        list_or_none(&generation.verified_claim_tokens),
        template
    );

    let mut request_prompt = prompt.clone();
    let mut last_error = None;
    for attempt in 1..=PLAN_ATTEMPTS {
        let result: Result<PlanResponse> = llm.json_with_images(
            &request_prompt,
            "You are an app-store creative director. Plan truthful, legible screenshot sets from supplied real product captures. Never invent features, awards, rankings, prices, or outcomes. Return JSON only.",
            &[contact_sheet.to_path_buf()],
        );
        match result.and_then(|mut response| {
            normalize_and_validate(
                &mut response.plans,
                spec,
                generation,
                variants,
                round,
                segment,
            )?;
            Ok(response.plans)
        }) {
            Ok(plans) => return Ok(plans),
            Err(error) => {
                let detail = format!("{error:#}");
                if attempt < PLAN_ATTEMPTS {
                    eprintln!("warning: generation plan rejected on attempt {attempt}/{PLAN_ATTEMPTS}: {detail}");
                    request_prompt = format!(
                        "{prompt}\n\n# Correction required\nThe previous plan was rejected: {detail}. Return a completely new JSON object that preserves every required plan, frame index, enum, color, and field."
                    );
                }
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("creative plan generation failed")))
}

pub fn write_manifest(
    path: &Path,
    round: usize,
    llm: &Llm,
    sources: &RawSourceCatalog,
    segment: &GenerationSegment,
    plans: &[CreativePlan],
) -> Result<()> {
    let manifest = GenerationManifest {
        round,
        generator_provider: llm.provider_label.to_string(),
        segment: Some(segment.clone()),
        raw_sources: sources
            .primary()
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        target_sources: sources
            .by_target
            .iter()
            .map(|(target, paths)| {
                (
                    target.clone(),
                    paths
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect(),
                )
            })
            .collect(),
        plans: plans.to_vec(),
    };
    std::fs::write(path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("failed to write generation manifest: {}", path.display()))
}

pub fn load_manifest(path: &Path) -> Result<GenerationManifest> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read generation manifest: {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse generation manifest: {}", path.display()))
}

pub fn render_plans(
    spec: &Spec,
    generation: &Generation,
    plans: &[CreativePlan],
    sources: &RawSourceCatalog,
    font_path: &Path,
    candidates_root: &Path,
) -> Result<()> {
    let font_bytes = std::fs::read(font_path)
        .with_context(|| format!("failed to read font: {}", font_path.display()))?;
    let font = FontArc::try_from_vec(font_bytes).map_err(|_| {
        anyhow::anyhow!(
            "font is not a supported TTF/OTF/TTC face: {}",
            font_path.display()
        )
    })?;
    std::fs::create_dir_all(candidates_root)?;
    for plan in plans {
        let candidate_dir = candidates_root.join(&plan.id);
        for target in &spec.targets {
            let target_sources = sources.for_target(&target.id);
            let target_dir = candidate_dir.join(&target.id);
            std::fs::create_dir_all(&target_dir)?;
            let (width, height) = target_size(target)?;
            match target.kind {
                AssetKind::Screenshot => {
                        for frame in &plan.frames {
                            let source =
                                image::open(&target_sources[frame.index - 1]).with_context(|| {
                                format!(
                                    "failed to open raw screenshot: {}",
                                    target_sources[frame.index - 1].display()
                                )
                            })?;
                            let mut rendered = render_screenshot(
                                width,
                                height,
                                &generation.brand_name,
                                frame,
                                &plan.palette,
                                &font,
                                target.store,
                                &source.to_rgba8(),
                            )?;
                        force_opaque(&mut rendered);
                        rendered.save(target_dir.join(format!("{:02}.png", frame.index)))?;
                    }
                }
                AssetKind::FeatureGraphic => {
                    let mut rendered = render_feature(
                        width,
                        height,
                        &generation.brand_name,
                        &plan.feature,
                        &plan.palette,
                        &font,
                        target.store,
                        sources.primary(),
                    )?;
                    force_opaque(&mut rendered);
                    rendered.save(target_dir.join("01.png"))?;
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct FrameRecipe {
    role: StoryRole,
    composition: Composition,
    decoration: Decoration,
}

fn story_roles(generation: &Generation) -> Result<Vec<StoryRole>> {
    if !generation.art_direction.story_roles.is_empty() {
        return generation
            .art_direction
            .story_roles
            .iter()
            .map(|role| story_role_from_id(role))
            .collect();
    }
    let count = generation.frame_count;
    Ok((0..count)
        .map(|index| {
            if index == 0 {
                StoryRole::Hero
            } else if index + 1 == count {
                StoryRole::Synthesis
            } else if index == 1 {
                StoryRole::Overview
            } else {
                StoryRole::Detail
            }
        })
        .collect())
}

fn frame_recipes(generation: &Generation, variant: usize) -> Result<Vec<FrameRecipe>> {
    let roles = story_roles(generation)?;
    let allowed_compositions = generation
        .art_direction
        .allowed_compositions
        .iter()
        .map(|value| composition_from_id(value))
        .collect::<Result<Vec<_>>>()?;
    let allowed_decorations = generation
        .art_direction
        .allowed_decorations
        .iter()
        .map(|value| decoration_from_id(value))
        .collect::<Result<Vec<_>>>()?;
    let max_repeated = generation.art_direction.max_consecutive_same_composition;

    let mut recipes = Vec::with_capacity(roles.len());
    let mut repeated = 0_usize;
    for (index, role) in roles.into_iter().enumerate() {
        let preferred = match role {
            StoryRole::Hero => Composition::EditorialHero,
            StoryRole::Overview => Composition::EditorialSplit,
            StoryRole::Detail => Composition::ChapterField,
            StoryRole::Proof => Composition::EditorialSplit,
            StoryRole::Synthesis => Composition::SynthesisDark,
            StoryRole::Legacy => Composition::Legacy,
        };
        let mut composition = if allowed_compositions.contains(&preferred) {
            preferred
        } else {
            allowed_compositions[(index + variant) % allowed_compositions.len()]
        };
        if recipes
            .last()
            .map(|recipe: &FrameRecipe| recipe.composition == composition)
            .unwrap_or(false)
        {
            repeated += 1;
        } else {
            repeated = 1;
        }
        if repeated > max_repeated {
            if let Some(alternative) = allowed_compositions
                .iter()
                .cycle()
                .skip(index + variant + 1)
                .take(allowed_compositions.len())
                .find(|candidate| **candidate != composition)
            {
                composition = *alternative;
                repeated = 1;
            }
        }
        recipes.push(FrameRecipe {
            role,
            composition,
            decoration: allowed_decorations[(index + variant) % allowed_decorations.len()],
        });
    }

    let mut unique = recipes
        .iter()
        .map(|recipe| recipe.composition)
        .collect::<HashSet<_>>();
    for (index, composition) in allowed_compositions.iter().enumerate() {
        if unique.len() >= generation.art_direction.min_unique_compositions {
            break;
        }
        if unique.insert(*composition) {
            let replace_index = (index + 1).min(recipes.len().saturating_sub(1));
            recipes[replace_index].composition = *composition;
        }
    }
    Ok(recipes)
}

fn frame_accent(generation: &Generation, frame_index: usize, variant: usize) -> String {
    let accents = &generation.art_direction.frame_accents;
    if accents.is_empty() {
        generation.palette[2].clone()
    } else {
        accents[(frame_index + variant) % accents.len()].clone()
    }
}

fn plan_template(
    generation: &Generation,
    variants: usize,
    round: usize,
    segment: &GenerationSegment,
) -> Result<String> {
    let layouts = generation
        .allowed_layouts
        .iter()
        .map(|layout| layout_from_id(layout))
        .collect::<Result<Vec<_>>>()?;
    let plans = (0..variants)
        .map(|variant| -> Result<serde_json::Value> {
            let family =
                &generation.creative_families[variant % generation.creative_families.len()];
            let recipes = frame_recipes(generation, variant)?;
            let frames = (0..generation.frame_count)
                .map(|index| {
                    let recipe = recipes[index];
                    let layout = if index == 0
                        && generation
                            .allowed_layouts
                            .iter()
                            .any(|layout| layout == "ui_dominant")
                    {
                        Layout::UiDominant
                    } else {
                        layouts[(index + variant) % layouts.len()]
                    };
                    serde_json::json!({
                        "index": index + 1,
                        "badge": "replace",
                        "headline": "replace",
                        "body": "replace",
                        "chips": ["replace"],
                        "layout": layout.id(),
                        "role": recipe.role.id(),
                        "composition": recipe.composition.id(),
                        "decoration": recipe.decoration.id(),
                        "accent": frame_accent(generation, index, variant),
                        "footer": "replace with a short payoff or reading axis"
                    })
                })
                .collect::<Vec<_>>();
            Ok(serde_json::json!({
                "id": format!("variant_{:02}", variant + 1),
                "family": family,
                "hypothesis_id": format!("r{round:02}-{}-{:02}", segment.id, variant + 1),
                "hypothesis": "replace with one falsifiable audience-and-intent hypothesis",
                "concept": "replace with the distinct creative hypothesis",
                "palette": {
                    "background_start": generation.palette[0],
                    "background_end": generation.palette[1],
                    "accent": generation.palette[2],
                    "text": generation.palette[3],
                    "muted": generation.palette.get(4).unwrap_or(&generation.palette[3])
                },
                "frames": frames,
                "feature": {
                    "headline": "replace",
                    "body": "replace",
                    "chips": ["replace"],
                    "source_indices": [1, generation.frame_count.min(2)]
                }
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(serde_json::to_string_pretty(
        &serde_json::json!({"plans": plans}),
    )?)
}

fn normalize_and_validate(
    plans: &mut [CreativePlan],
    spec: &Spec,
    generation: &Generation,
    variants: usize,
    round: usize,
    segment: &GenerationSegment,
) -> Result<()> {
    anyhow::ensure!(
        plans.len() == variants,
        "expected {variants} plans, found {}",
        plans.len()
    );
    let allowed_colors = generation
        .palette
        .iter()
        .chain(generation.art_direction.frame_accents.iter())
        .cloned()
        .collect::<HashSet<_>>();
    let allowed_layouts = generation
        .allowed_layouts
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    for (plan_index, plan) in plans.iter_mut().enumerate() {
        let recipes = frame_recipes(generation, plan_index)?;
        plan.id = format!("variant_{:02}", plan_index + 1);
        let expected_family =
            &generation.creative_families[plan_index % generation.creative_families.len()];
        anyhow::ensure!(
            plan.family.id() == expected_family,
            "{} must use assigned family {expected_family}",
            plan.id
        );
        plan.hypothesis_id = format!("r{round:02}-{}-{:02}", segment.id, plan_index + 1);
        anyhow::ensure!(
            !plan.hypothesis.trim().is_empty(),
            "{} hypothesis is empty",
            plan.id
        );
        anyhow::ensure!(
            !plan.concept.trim().is_empty(),
            "{} concept is empty",
            plan.id
        );
        for color in [
            &plan.palette.background_start,
            &plan.palette.background_end,
            &plan.palette.accent,
            &plan.palette.text,
            &plan.palette.muted,
        ] {
            anyhow::ensure!(
                allowed_colors.contains(color),
                "{} uses color outside generation.palette: {color}",
                plan.id
            );
        }
        anyhow::ensure!(
            plan.frames.len() == generation.frame_count,
            "{} expected {} frames, found {}",
            plan.id,
            generation.frame_count,
            plan.frames.len()
        );
        plan.frames.sort_by_key(|frame| frame.index);
        for (expected_index, frame) in plan.frames.iter_mut().enumerate() {
            anyhow::ensure!(
                frame.index == expected_index + 1,
                "{} frame indices must be exactly 1..{}",
                plan.id,
                generation.frame_count
            );
            let recipe = recipes[expected_index];
            frame.role = recipe.role;
            frame.composition = recipe.composition;
            frame.decoration = recipe.decoration;
            frame.accent = Some(frame_accent(generation, expected_index, plan_index));
            anyhow::ensure!(
                !frame.badge.trim().is_empty(),
                "{} frame {} badge is empty",
                plan.id,
                frame.index
            );
            anyhow::ensure!(
                !frame.headline.trim().is_empty(),
                "{} frame {} headline is empty",
                plan.id,
                frame.index
            );
            anyhow::ensure!(
                !frame.body.trim().is_empty(),
                "{} frame {} body is empty",
                plan.id,
                frame.index
            );
            anyhow::ensure!(
                frame.chips.len() <= 3,
                "{} frame {} has more than three chips",
                plan.id,
                frame.index
            );
            anyhow::ensure!(
                allowed_layouts.contains(frame.layout.id()),
                "{} frame {} uses a disallowed layout",
                plan.id,
                frame.index
            );
            anyhow::ensure!(
                generation
                    .art_direction
                    .allowed_compositions
                    .iter()
                    .any(|value| value == frame.composition.id()),
                "{} frame {} uses a disallowed composition",
                plan.id,
                frame.index
            );
            anyhow::ensure!(
                generation
                    .art_direction
                    .allowed_decorations
                    .iter()
                    .any(|value| value == frame.decoration.id()),
                "{} frame {} uses a disallowed decoration",
                plan.id,
                frame.index
            );
            anyhow::ensure!(
                frame
                    .accent
                    .as_ref()
                    .map(|color| allowed_colors.contains(color))
                    .unwrap_or(false),
                "{} frame {} uses a disallowed accent",
                plan.id,
                frame.index
            );
            anyhow::ensure!(
                !frame.footer.trim().is_empty(),
                "{} frame {} footer is empty",
                plan.id,
                frame.index
            );
        }
        let unique_compositions = plan
            .frames
            .iter()
            .map(|frame| frame.composition)
            .collect::<HashSet<_>>()
            .len();
        anyhow::ensure!(
            unique_compositions >= generation.art_direction.min_unique_compositions,
            "{} needs at least {} unique compositions",
            plan.id,
            generation.art_direction.min_unique_compositions
        );
        let max_run = max_composition_run(&plan.frames);
        anyhow::ensure!(
            max_run <= generation.art_direction.max_consecutive_same_composition,
            "{} repeats one composition {} times",
            plan.id,
            max_run
        );
        if generation
            .allowed_layouts
            .iter()
            .any(|layout| layout == "ui_dominant")
        {
            anyhow::ensure!(
                plan.frames.first().map(|frame| frame.layout) == Some(Layout::UiDominant),
                "{} first frame must use ui_dominant",
                plan.id
            );
        }
        anyhow::ensure!(
            !plan.feature.headline.trim().is_empty(),
            "{} feature headline is empty",
            plan.id
        );
        anyhow::ensure!(
            !plan.feature.body.trim().is_empty(),
            "{} feature body is empty",
            plan.id
        );
        anyhow::ensure!(
            plan.feature.chips.len() <= 3,
            "{} feature has more than three chips",
            plan.id
        );
        anyhow::ensure!(
            !plan.feature.source_indices.is_empty() && plan.feature.source_indices.len() <= 2,
            "{} feature needs one or two source indices",
            plan.id
        );
        anyhow::ensure!(
            plan.feature
                .source_indices
                .iter()
                .all(|index| (1..=generation.frame_count).contains(index)),
            "{} feature contains an invalid source index",
            plan.id
        );
        validate_copy_claims(spec, generation, plan)?;
    }
    Ok(())
}

fn max_composition_run(frames: &[FramePlan]) -> usize {
    let mut longest = 0_usize;
    let mut current = 0_usize;
    let mut previous = None;
    for frame in frames {
        if previous == Some(frame.composition) {
            current += 1;
        } else {
            previous = Some(frame.composition);
            current = 1;
        }
        longest = longest.max(current);
    }
    longest
}

fn validate_copy_claims(spec: &Spec, generation: &Generation, plan: &CreativePlan) -> Result<()> {
    let mut values = Vec::new();
    for frame in &plan.frames {
        values.extend([
            frame.badge.as_str(),
            frame.headline.as_str(),
            frame.body.as_str(),
        ]);
        values.extend(frame.chips.iter().map(String::as_str));
    }
    values.extend([plan.feature.headline.as_str(), plan.feature.body.as_str()]);
    values.extend(plan.feature.chips.iter().map(String::as_str));
    let copy = values.join(" ").to_lowercase();

    for prohibited in &spec.prohibited_claims {
        anyhow::ensure!(
            !copy.contains(&prohibited.to_lowercase()),
            "{} contains prohibited claim: {prohibited}",
            plan.id
        );
    }
    let avoid_terms = generation
        .store_tone_profiles
        .apple
        .avoid_phrases
        .iter()
        .chain(generation.store_tone_profiles.google.avoid_phrases.iter());
    for term in avoid_terms {
        if !term.trim().is_empty() {
            let avoid = term.to_lowercase();
            anyhow::ensure!(
                !copy.contains(&avoid),
                "{} contains tone-ban phrase: {term}",
                plan.id
            );
        }
    }

    const TRUST_MARKERS: &[&str] = &[
        "#1",
        "1위",
        "최고",
        "best",
        "award",
        "수상",
        "editor's choice",
        "에디터 추천",
        "평점",
        "별점",
        "★",
        "stars",
        "%",
        "guarantee",
        "보장",
        "million",
        "만 명",
        "downloads",
        "다운로드",
    ];
    let present_markers = TRUST_MARKERS
        .iter()
        .filter(|marker| copy.contains(**marker))
        .copied()
        .collect::<Vec<_>>();
    if !present_markers.is_empty() {
        let has_verified_token = generation
            .verified_claim_tokens
            .iter()
            .any(|token| copy.contains(&token.to_lowercase()));
        anyhow::ensure!(
            has_verified_token,
            "{} contains unverified trust marker(s): {}",
            plan.id,
            present_markers.join(", ")
        );
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct StoreRenderProfile {
    deco_intensity: f32,
    shadow_alpha: u8,
    stroke_alpha: u8,
    stroke_width: i32,
}

fn store_render_profile(store: Store) -> StoreRenderProfile {
    match store {
        Store::Apple => StoreRenderProfile {
            deco_intensity: 0.78,
            shadow_alpha: 88,
            stroke_alpha: 85,
            stroke_width: 1,
        },
        Store::Google => StoreRenderProfile {
            deco_intensity: 1.10,
            shadow_alpha: 118,
            stroke_alpha: 125,
            stroke_width: 2,
        },
    }
}

fn render_screenshot(
    width: u32,
    height: u32,
    brand: &str,
    frame: &FramePlan,
    palette: &PalettePlan,
    font: &FontArc,
    store: Store,
    source: &RgbaImage,
) -> Result<RgbaImage> {
    if frame.composition != Composition::Legacy {
        return render_art_directed_screenshot(
            width,
            height,
            brand,
            frame,
            palette,
            font,
            source,
            store,
        );
    }
    let profile = store_render_profile(store);
    let start = parse_color(&palette.background_start)?;
    let end = parse_color(&palette.background_end)?;
    let accent = parse_color(&palette.accent)?;
    let text = parse_color(&palette.text)?;
    let muted = parse_color(&palette.muted)?;
    let mut canvas = gradient(width, height, start, end);
    add_decorations(&mut canvas, accent, profile.deco_intensity);
    let margin = (width as f32 * 0.075) as i32;
    let brand_scale = PxScale::from(width as f32 * 0.026);
    draw_text_mut(
        &mut canvas,
        accent,
        margin,
        (height as f32 * 0.035) as i32,
        brand_scale,
        font,
        brand,
    );
    let badge_y = (height as f32 * 0.09) as i32;
    draw_pill(
        &mut canvas,
        font,
        margin,
        badge_y,
        &frame.badge,
        width as f32 * 0.023,
        accent,
        start,
    );

    let headline_scale = PxScale::from(width as f32 * if width > 1600 { 0.055 } else { 0.065 });
    let headline_y = (height as f32 * 0.14) as i32;
    let headline_width = width.saturating_sub((margin as u32) * 2);
    let headline_lines = wrap_text(font, headline_scale, &frame.headline, headline_width, 2);
    let after_headline = draw_lines(
        &mut canvas,
        font,
        headline_scale,
        text,
        margin,
        headline_y,
        &headline_lines,
        1.12,
    );
    let body_scale = PxScale::from(width as f32 * 0.025);
    let body_lines = wrap_text(font, body_scale, &frame.body, headline_width, 2);
    let after_body = draw_lines(
        &mut canvas,
        font,
        body_scale,
        muted,
        margin,
        after_headline + (height as f32 * 0.018) as i32,
        &body_lines,
        1.25,
    );
    let chips_y = after_body + (height as f32 * 0.018) as i32;
    draw_chips(
        &mut canvas,
        font,
        margin,
        chips_y,
        &frame.chips,
        width,
        accent,
        start,
    );
    let screenshot_y = match frame.layout {
        Layout::UiDominant => (height as f32 * 0.27) as i32,
        Layout::DeviceBottom => (height as f32 * 0.37) as i32,
        Layout::DeviceCenter => (height as f32 * 0.33) as i32,
        Layout::UiFocus => (height as f32 * 0.31) as i32,
    };
    let max_width = (width as f32
        * match frame.layout {
            Layout::UiDominant => 0.92,
            Layout::DeviceBottom => 0.68,
            Layout::DeviceCenter => 0.76,
            Layout::UiFocus => 0.86,
        }) as u32;
    let max_height =
        height.saturating_sub(screenshot_y.max(0) as u32 + (height as f32 * 0.035) as u32);
    let fitted = fit_image(source, max_width, max_height);
    let x = ((width.saturating_sub(fitted.width())) / 2) as i32;
    paste_device(
        &mut canvas,
        &fitted,
        x,
        screenshot_y,
        (width as f32 * 0.015) as u32,
        accent,
        profile,
    );
    Ok(canvas)
}

#[allow(clippy::too_many_arguments)]
fn render_art_directed_screenshot(
    width: u32,
    height: u32,
    brand: &str,
    frame: &FramePlan,
    palette: &PalettePlan,
    font: &FontArc,
    source: &RgbaImage,
    store: Store,
) -> Result<RgbaImage> {
    let dark = parse_color(&palette.background_start)?;
    let dark_end = parse_color(&palette.background_end)?;
    let text = parse_color(&palette.text)?;
    let muted = parse_color(&palette.muted)?;
    let accent = frame
        .accent
        .as_deref()
        .map(parse_color)
        .transpose()?
        .unwrap_or(parse_color(&palette.accent)?);
    let paper = mix_color(dark, text, 0.94);
    let ink = mix_color(dark, Rgba([0, 0, 0, 255]), 0.35);
    let profile = store_render_profile(store);
    let margin = (width as f32 * 0.064) as i32;
    let wide = width as f32 / height as f32 > 0.62;

    let mut canvas = match frame.composition {
        Composition::EditorialHero | Composition::SynthesisDark => {
            gradient(width, height, dark, dark_end)
        }
        Composition::EditorialSplit | Composition::ChapterField => {
            RgbaImage::from_pixel(width, height, paper)
        }
        Composition::Legacy => unreachable!("legacy composition uses the legacy renderer"),
    };

    match frame.composition {
        Composition::EditorialHero => {
            draw_art_brand(&mut canvas, font, brand, frame, margin, text, muted);
            let copy_bottom = draw_art_copy(
                &mut canvas,
                font,
                frame,
                margin,
                (height as f32 * 0.14) as i32,
                (width as f32 * if wide { 0.43 } else { 0.82 }) as u32,
                text,
                muted,
                accent,
            );
            draw_art_decoration(
                &mut canvas,
                frame.decoration,
                accent,
                dark_end,
                (margin, copy_bottom + (height as f32 * 0.035) as i32),
            );
            let (max_width, max_height, x, y) = if wide {
                let max_width = (width as f32 * 0.45) as u32;
                let max_height = (height as f32 * 0.84) as u32;
                let fitted = fit_image(source, max_width, max_height);
                (
                    max_width,
                    max_height,
                    width as i32 - fitted.width() as i32 - margin,
                    (height.saturating_sub(fitted.height()) / 2) as i32,
                )
            } else {
                (
                    (width as f32 * 0.82) as u32,
                    (height as f32 * 0.62) as u32,
                    0,
                    (height as f32 * 0.37) as i32,
                )
            };
            let fitted = fit_image(source, max_width, max_height);
            let target_x = if wide {
                x
            } else {
                ((width - fitted.width()) / 2) as i32
            };
            paste_device(
                &mut canvas,
                &fitted,
                target_x,
                y,
                (width as f32 * 0.015) as u32,
                accent,
                profile,
            );
        }
        Composition::EditorialSplit => {
            let circle_radius = (width as f32 * 0.23) as i32;
            draw_filled_circle_mut(
                &mut canvas,
                (width as i32 - circle_radius / 3, circle_radius / 2),
                circle_radius,
                mix_color(paper, accent, 0.82),
            );
            draw_art_brand(&mut canvas, font, brand, frame, margin, ink, muted);
            let copy_width = if wide { 0.42 } else { 0.82 };
            let copy_bottom = draw_art_copy(
                &mut canvas,
                font,
                frame,
                margin,
                (height as f32 * 0.14) as i32,
                (width as f32 * copy_width) as u32,
                ink,
                mix_color(ink, paper, 0.42),
                accent,
            );
            draw_art_decoration(
                &mut canvas,
                frame.decoration,
                accent,
                paper,
                (
                    margin,
                    copy_bottom + (height as f32 * if wide { 0.045 } else { 0.025 }) as i32,
                ),
            );
            let (max_width, max_height, y) = if wide {
                (
                    (width as f32 * 0.43) as u32,
                    (height as f32 * 0.82) as u32,
                    (height as f32 * 0.11) as i32,
                )
            } else {
                (
                    (width as f32 * 0.78) as u32,
                    (height as f32 * 0.60) as u32,
                    (height as f32 * 0.39) as i32,
                )
            };
            let fitted = fit_image(source, max_width, max_height);
            let x = if wide {
                width as i32 - fitted.width() as i32 - margin
            } else {
                ((width - fitted.width()) / 2) as i32
            };
            paste_device(
                &mut canvas,
                &fitted,
                x,
                y,
                (width as f32 * 0.015) as u32,
                accent,
                profile,
            );
        }
        Composition::ChapterField => {
            let field_height = (height as f32 * 0.51) as i32;
            draw_filled_rect_mut(
                &mut canvas,
                Rect::at(0, 0).of_size(width, field_height as u32),
                accent,
            );
            draw_polygon_mut(
                &mut canvas,
                &[
                    Point::new((width as f32 * 0.74) as i32, 0),
                    Point::new(width as i32, 0),
                    Point::new(width as i32, field_height),
                    Point::new((width as f32 * 0.48) as i32, field_height),
                ],
                paper,
            );
            let index_text = format!("{:02}", frame.index);
            draw_text_mut(
                &mut canvas,
                mix_color(paper, accent, 0.18),
                (width as f32 * 0.73) as i32,
                (height as f32 * 0.10) as i32,
                PxScale::from(width as f32 * 0.22),
                font,
                &index_text,
            );
            draw_art_decoration(
                &mut canvas,
                frame.decoration,
                accent,
                paper,
                ((width as f32 * 0.72) as i32, (height as f32 * 0.31) as i32),
            );
            draw_art_brand(
                &mut canvas,
                font,
                brand,
                frame,
                margin,
                paper,
                mix_color(paper, accent, 0.25),
            );
            draw_art_copy(
                &mut canvas,
                font,
                frame,
                margin,
                (height as f32 * 0.145) as i32,
                (width as f32 * 0.58) as u32,
                paper,
                mix_color(paper, accent, 0.20),
                paper,
            );
            let fitted = fit_image(
                source,
                (width as f32 * if wide { 0.74 } else { 0.86 }) as u32,
                (height as f32 * 0.34) as u32,
            );
            let x = ((width - fitted.width()) / 2) as i32;
            let y = (height as f32 * 0.405) as i32;
            paste_device(
                &mut canvas,
                &fitted,
                x,
                y,
                (width as f32 * 0.014) as u32,
                accent,
                profile,
            );
            let footer_y = (y + fitted.height() as i32 + (height as f32 * 0.065) as i32)
                .min((height as f32 * 0.82) as i32);
            draw_line_segment_mut(
                &mut canvas,
                (margin as f32, footer_y as f32),
                ((width as i32 - margin) as f32, footer_y as f32),
                mix_color(paper, ink, 0.28),
            );
            draw_text_mut(
                &mut canvas,
                accent,
                margin,
                footer_y + (height as f32 * 0.022) as i32,
                PxScale::from(width as f32 * 0.018),
                font,
                frame.role.id(),
            );
            draw_text_mut(
                &mut canvas,
                ink,
                margin,
                footer_y + (height as f32 * 0.052) as i32,
                PxScale::from(width as f32 * 0.036),
                font,
                &frame.footer,
            );
        }
        Composition::SynthesisDark => {
            draw_art_brand(&mut canvas, font, brand, frame, margin, text, muted);
            draw_art_copy(
                &mut canvas,
                font,
                frame,
                margin,
                (height as f32 * 0.14) as i32,
                (width as f32 * if wide { 0.42 } else { 0.82 }) as u32,
                text,
                muted,
                accent,
            );
            draw_art_decoration(
                &mut canvas,
                frame.decoration,
                accent,
                dark_end,
                (margin, (height as f32 * 0.62) as i32),
            );
            let fitted = fit_image(
                source,
                (width as f32 * if wide { 0.45 } else { 0.76 }) as u32,
                (height as f32 * if wide { 0.82 } else { 0.61 }) as u32,
            );
            let x = width as i32 - fitted.width() as i32 - margin;
            let y = if wide {
                (height.saturating_sub(fitted.height()) / 2) as i32
            } else {
                (height as f32 * 0.37) as i32
            };
            paste_device(
                &mut canvas,
                &fitted,
                x,
                y,
                (width as f32 * 0.015) as u32,
                accent,
                profile,
            );
        }
        Composition::Legacy => unreachable!(),
    }
    Ok(canvas)
}

#[allow(clippy::too_many_arguments)]
fn draw_art_copy(
    canvas: &mut RgbaImage,
    font: &FontArc,
    frame: &FramePlan,
    x: i32,
    y: i32,
    max_width: u32,
    headline_color: Rgba<u8>,
    body_color: Rgba<u8>,
    label_color: Rgba<u8>,
) -> i32 {
    let width = canvas.width();
    let height = canvas.height();
    let label_scale = PxScale::from(width as f32 * 0.019);
    draw_text_mut(canvas, label_color, x, y, label_scale, font, &frame.badge);
    let headline_scale = PxScale::from(width as f32 * if width > 1600 { 0.052 } else { 0.064 });
    let headline_lines = wrap_text(font, headline_scale, &frame.headline, max_width, 2);
    let after_headline = draw_lines(
        canvas,
        font,
        headline_scale,
        headline_color,
        x,
        y + (height as f32 * 0.043) as i32,
        &headline_lines,
        1.10,
    );
    let body_scale = PxScale::from(width as f32 * 0.024);
    let body_lines = wrap_text(font, body_scale, &frame.body, max_width, 2);
    let after_body = draw_lines(
        canvas,
        font,
        body_scale,
        body_color,
        x,
        after_headline + (height as f32 * 0.019) as i32,
        &body_lines,
        1.24,
    );
    if !frame.chips.is_empty() {
        draw_text_mut(
            canvas,
            label_color,
            x,
            after_body + (height as f32 * 0.018) as i32,
            PxScale::from(width as f32 * 0.019),
            font,
            &frame.chips.join("  ·  "),
        );
    }
    after_body
}

fn draw_art_brand(
    canvas: &mut RgbaImage,
    font: &FontArc,
    brand: &str,
    frame: &FramePlan,
    margin: i32,
    color: Rgba<u8>,
    muted: Rgba<u8>,
) {
    let width = canvas.width();
    let height = canvas.height();
    draw_text_mut(
        canvas,
        color,
        margin,
        (height as f32 * 0.035) as i32,
        PxScale::from(width as f32 * 0.026),
        font,
        brand,
    );
    draw_text_mut(
        canvas,
        muted,
        margin,
        (height as f32 * 0.066) as i32,
        PxScale::from(width as f32 * 0.014),
        font,
        frame.role.id(),
    );
    let number = format!("{:02}", frame.index);
    let number_width = text_size(PxScale::from(width as f32 * 0.018), font, &number).0 as i32;
    draw_text_mut(
        canvas,
        muted,
        width as i32 - margin - number_width,
        (height as f32 * 0.045) as i32,
        PxScale::from(width as f32 * 0.018),
        font,
        &number,
    );
}

fn draw_art_decoration(
    canvas: &mut RgbaImage,
    decoration: Decoration,
    accent: Rgba<u8>,
    background: Rgba<u8>,
    anchor: (i32, i32),
) {
    let width = canvas.width() as f32;
    let height = canvas.height() as f32;
    let (x, y) = anchor;
    match decoration {
        Decoration::None => {}
        Decoration::Spectrum => {
            let bar_width = (width * 0.065) as u32;
            let bar_height = (height * 0.004).max(4.0) as u32;
            for index in 0..5 {
                draw_filled_rect_mut(
                    canvas,
                    Rect::at(x + index * (bar_width as i32 + 7), y).of_size(bar_width, bar_height),
                    mix_color(background, accent, 0.36 + index as f32 * 0.12),
                );
            }
        }
        Decoration::Orbit => {
            let center = (x + (width * 0.10) as i32, y + (height * 0.035) as i32);
            for index in 1..=3 {
                draw_hollow_circle_mut(
                    canvas,
                    center,
                    (width * (0.035 + index as f32 * 0.025)) as i32,
                    mix_color(background, accent, 0.18 + index as f32 * 0.12),
                );
            }
            draw_line_segment_mut(
                canvas,
                (
                    center.0 as f32 - width * 0.10,
                    center.1 as f32 + height * 0.02,
                ),
                (
                    center.0 as f32 + width * 0.13,
                    center.1 as f32 - height * 0.025,
                ),
                mix_color(background, accent, 0.55),
            );
        }
        Decoration::Grid => {
            for index in 0..5 {
                let offset = index as f32 * height * 0.012;
                draw_line_segment_mut(
                    canvas,
                    (x as f32, y as f32 + offset),
                    (x as f32 + width * 0.23, y as f32 - height * 0.035 + offset),
                    mix_color(background, accent, 0.22 + index as f32 * 0.07),
                );
            }
        }
        Decoration::Signal => {
            for index in 0..6 {
                let line_width = width * (0.045 + (index % 3) as f32 * 0.025);
                let line_y = y as f32 + index as f32 * height * 0.013;
                draw_line_segment_mut(
                    canvas,
                    (x as f32, line_y),
                    (x as f32 + line_width, line_y),
                    mix_color(background, accent, 0.34 + index as f32 * 0.07),
                );
            }
        }
    }
}

fn mix_color(base: Rgba<u8>, overlay: Rgba<u8>, amount: f32) -> Rgba<u8> {
    let amount = amount.clamp(0.0, 1.0);
    Rgba([
        lerp(base[0], overlay[0], amount),
        lerp(base[1], overlay[1], amount),
        lerp(base[2], overlay[2], amount),
        255,
    ])
}

fn render_feature(
    width: u32,
    height: u32,
    brand: &str,
    feature: &FeaturePlan,
    palette: &PalettePlan,
    font: &FontArc,
    store: Store,
    sources: &[PathBuf],
) -> Result<RgbaImage> {
    let start = parse_color(&palette.background_start)?;
    let end = parse_color(&palette.background_end)?;
    let accent = parse_color(&palette.accent)?;
    let text = parse_color(&palette.text)?;
    let muted = parse_color(&palette.muted)?;
    let profile = store_render_profile(store);
    let mut canvas = gradient(width, height, start, end);
    add_decorations(&mut canvas, accent, profile.deco_intensity);
    let margin = (width as f32 * 0.055) as i32;
    draw_text_mut(
        &mut canvas,
        accent,
        margin,
        (height as f32 * 0.08) as i32,
        PxScale::from(height as f32 * 0.055),
        font,
        brand,
    );
    let headline_lines = wrap_text(
        font,
        PxScale::from(height as f32 * 0.105),
        &feature.headline,
        (width as f32 * 0.52) as u32,
        2,
    );
    let after_headline = draw_lines(
        &mut canvas,
        font,
        PxScale::from(height as f32 * 0.105),
        text,
        margin,
        (height as f32 * 0.25) as i32,
        &headline_lines,
        1.08,
    );
    let body_lines = wrap_text(
        font,
        PxScale::from(height as f32 * 0.042),
        &feature.body,
        (width as f32 * 0.52) as u32,
        2,
    );
    let after_body = draw_lines(
        &mut canvas,
        font,
        PxScale::from(height as f32 * 0.042),
        muted,
        margin,
        after_headline + (height as f32 * 0.035) as i32,
        &body_lines,
        1.2,
    );
    draw_chips(
        &mut canvas,
        font,
        margin,
        after_body + (height as f32 * 0.035) as i32,
        &feature.chips,
        width / 2,
        accent,
        start,
    );

    let image_width = (width as f32 * 0.22) as u32;
    for (position, source_index) in feature.source_indices.iter().enumerate() {
        let source = image::open(&sources[*source_index - 1])?.to_rgba8();
        let max_height = (height as f32 * 0.82) as u32;
        let fitted = fit_image(&source, image_width, max_height);
        let x = (width as f32 * (0.61 + position as f32 * 0.17)) as i32;
        let y = (height.saturating_sub(fitted.height()) / 2) as i32 + (position as i32 * 14 - 7);
        paste_device(
            &mut canvas,
            &fitted,
            x,
            y,
            (height as f32 * 0.025) as u32,
            accent,
            profile,
        );
    }
    Ok(canvas)
}

fn target_size(target: &Target) -> Result<(u32, u32)> {
    if let Some(size) = target.allowed_sizes.first() {
        return Ok((size.width, size.height));
    }
    if target.kind == AssetKind::FeatureGraphic {
        return Ok((1024, 500));
    }
    anyhow::bail!(
        "generation target {} needs at least one allowed_sizes entry",
        target.id
    )
}

fn fit_image(source: &RgbaImage, max_width: u32, max_height: u32) -> RgbaImage {
    let scale = (max_width as f64 / source.width() as f64)
        .min(max_height as f64 / source.height() as f64)
        .min(1.0_f64.max(max_width as f64 / source.width() as f64));
    let width = (source.width() as f64 * scale).round().max(1.0) as u32;
    let height = (source.height() as f64 * scale).round().max(1.0) as u32;
    image::imageops::resize(source, width, height, FilterType::Lanczos3)
}

fn paste_device(
    canvas: &mut RgbaImage,
    source: &RgbaImage,
    x: i32,
    y: i32,
    radius: u32,
    accent: Rgba<u8>,
    profile: StoreRenderProfile,
) {
    let border = (source.width() as f32 * 0.012).round().max(5.0) as i32;
    let outer_width = source.width() as i32 + border * 2;
    let outer_height = source.height() as i32 + border * 2;
    fill_rounded_blended(
        canvas,
        x + 16,
        y + 24,
        outer_width,
        outer_height,
        radius as i32 + border,
        Rgba([0, 0, 0, profile.shadow_alpha]),
    );
    fill_rounded_blended(
        canvas,
        x - border,
        y - border,
        outer_width,
        outer_height,
        radius as i32 + border,
        Rgba([12, 13, 18, 255]),
    );
    for source_y in 0..source.height() {
        for source_x in 0..source.width() {
            if inside_rounded(
                source_x as i32,
                source_y as i32,
                source.width() as i32,
                source.height() as i32,
                radius as i32,
            ) {
                let target_x = x + source_x as i32;
                let target_y = y + source_y as i32;
                if target_x >= 0
                    && target_y >= 0
                    && target_x < canvas.width() as i32
                    && target_y < canvas.height() as i32
                {
                    blend_pixel(
                        canvas.get_pixel_mut(target_x as u32, target_y as u32),
                        *source.get_pixel(source_x, source_y),
                    );
                }
            }
        }
    }
    let line_width = profile.stroke_width.max(1);
    for offset in 0..line_width {
        let color = Rgba([accent[0], accent[1], accent[2], profile.stroke_alpha]);
        stroke_rounded(
            canvas,
            x - border + offset,
            y - border + offset,
            outer_width - offset * 2,
            outer_height - offset * 2,
            radius as i32 + border,
            color,
        );
    }
}

fn gradient(width: u32, height: u32, start: Rgba<u8>, end: Rgba<u8>) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);
    for y in 0..height {
        let t = if height <= 1 {
            0.0
        } else {
            y as f32 / (height - 1) as f32
        };
        let color = Rgba([
            lerp(start[0], end[0], t),
            lerp(start[1], end[1], t),
            lerp(start[2], end[2], t),
            255,
        ]);
        for x in 0..width {
            image.put_pixel(x, y, color);
        }
    }
    image
}

fn add_decorations(canvas: &mut RgbaImage, accent: Rgba<u8>, deco_intensity: f32) {
    let deco_intensity = deco_intensity.clamp(0.4, 1.8);
    let width = canvas.width() as i32;
    let height = canvas.height() as i32;
    for index in 0..5 {
        let radius = (width as f32 * (0.16 + index as f32 * 0.035)) as i32;
        let base_alpha = (12_u16.saturating_sub((index * 2) as u16) as f32) * deco_intensity;
        let alpha = base_alpha.clamp(1.0, 24.0).round() as u8;
        let color = Rgba([accent[0], accent[1], accent[2], alpha]);
        fill_rounded_blended(
            canvas,
            width - radius,
            -radius / 2 + index as i32 * radius / 3,
            radius * 2,
            radius * 2,
            radius,
            color,
        );
    }
    let bar_width = (width as f32 * 0.17) as u32;
    let bar_height = (height as f32 * 0.004).max(3.0) as u32;
    let bar_alpha = (255.0 * (0.10 * deco_intensity)).clamp(35.0, 160.0).round() as u8;
    draw_filled_rect_mut(
        canvas,
        Rect::at((width as f32 * 0.075) as i32, (height as f32 * 0.12) as i32)
            .of_size(bar_width, bar_height),
        Rgba([accent[0], accent[1], accent[2], bar_alpha]),
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_pill(
    canvas: &mut RgbaImage,
    font: &FontArc,
    x: i32,
    y: i32,
    label: &str,
    size: f32,
    accent: Rgba<u8>,
    background: Rgba<u8>,
) -> i32 {
    let scale = PxScale::from(size);
    let (text_width, text_height) = text_size(scale, font, label);
    let pad_x = (size * 0.85) as i32;
    let height = (text_height as f32 * 1.75) as i32;
    let width = text_width as i32 + pad_x * 2;
    pill_shape(
        canvas,
        x,
        y,
        width,
        height,
        Rgba([background[0], background[1], background[2], 230]),
    );
    draw_text_mut(
        canvas,
        accent,
        x + pad_x,
        y + (height - text_height as i32) / 2,
        scale,
        font,
        label,
    );
    width
}

#[allow(clippy::too_many_arguments)]
fn draw_chips(
    canvas: &mut RgbaImage,
    font: &FontArc,
    start_x: i32,
    y: i32,
    chips: &[String],
    max_width: u32,
    accent: Rgba<u8>,
    background: Rgba<u8>,
) {
    let mut x = start_x;
    let size = max_width as f32 * 0.026;
    for chip in chips.iter().take(3) {
        let width = draw_pill(canvas, font, x, y, chip, size, accent, background);
        x += width + (size * 0.55) as i32;
        if x > max_width as i32 - start_x {
            break;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_lines(
    canvas: &mut RgbaImage,
    font: &FontArc,
    scale: PxScale,
    color: Rgba<u8>,
    x: i32,
    y: i32,
    lines: &[String],
    line_height: f32,
) -> i32 {
    let mut cursor = y;
    for line in lines {
        draw_text_mut(canvas, color, x, cursor, scale, font, line);
        cursor += (scale.y * line_height) as i32;
    }
    cursor
}

fn wrap_text(
    font: &FontArc,
    scale: PxScale,
    text: &str,
    max_width: u32,
    max_lines: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if text_size(scale, font, &candidate).0 <= max_width {
                current = candidate;
            } else {
                if !current.is_empty() {
                    lines.push(current);
                }
                current = word.to_string();
            }
            if lines.len() + 1 >= max_lines {
                break;
            }
        }
        if !current.is_empty() && lines.len() < max_lines {
            lines.push(current);
        }
        if lines.len() >= max_lines {
            break;
        }
    }
    if lines.is_empty() {
        lines.push(text.to_string());
    }
    lines
}

fn pill_shape(canvas: &mut RgbaImage, x: i32, y: i32, width: i32, height: i32, color: Rgba<u8>) {
    let radius = height / 2;
    draw_filled_rect_mut(
        canvas,
        Rect::at(x + radius, y).of_size((width - radius * 2).max(1) as u32, height.max(1) as u32),
        color,
    );
    draw_filled_circle_mut(canvas, (x + radius, y + radius), radius, color);
    draw_filled_circle_mut(canvas, (x + width - radius, y + radius), radius, color);
}

fn fill_rounded_blended(
    canvas: &mut RgbaImage,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    radius: i32,
    color: Rgba<u8>,
) {
    for local_y in 0..height.max(0) {
        for local_x in 0..width.max(0) {
            if inside_rounded(local_x, local_y, width, height, radius) {
                let target_x = x + local_x;
                let target_y = y + local_y;
                if target_x >= 0
                    && target_y >= 0
                    && target_x < canvas.width() as i32
                    && target_y < canvas.height() as i32
                {
                    blend_pixel(
                        canvas.get_pixel_mut(target_x as u32, target_y as u32),
                        color,
                    );
                }
            }
        }
    }
}

fn stroke_rounded(
    canvas: &mut RgbaImage,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    radius: i32,
    color: Rgba<u8>,
) {
    for local_y in 0..height.max(0) {
        for local_x in 0..width.max(0) {
            let outer = inside_rounded(local_x, local_y, width, height, radius);
            let inner = inside_rounded(
                local_x - 2,
                local_y - 2,
                width - 4,
                height - 4,
                (radius - 2).max(0),
            );
            if outer && !inner {
                let target_x = x + local_x;
                let target_y = y + local_y;
                if target_x >= 0
                    && target_y >= 0
                    && target_x < canvas.width() as i32
                    && target_y < canvas.height() as i32
                {
                    blend_pixel(
                        canvas.get_pixel_mut(target_x as u32, target_y as u32),
                        color,
                    );
                }
            }
        }
    }
}

fn inside_rounded(x: i32, y: i32, width: i32, height: i32, radius: i32) -> bool {
    if width <= 0 || height <= 0 || x < 0 || y < 0 || x >= width || y >= height {
        return false;
    }
    let radius = radius.min(width / 2).min(height / 2).max(0);
    let corner_x = if x < radius {
        radius
    } else if x >= width - radius {
        width - radius - 1
    } else {
        x
    };
    let corner_y = if y < radius {
        radius
    } else if y >= height - radius {
        height - radius - 1
    } else {
        y
    };
    let dx = x - corner_x;
    let dy = y - corner_y;
    dx * dx + dy * dy <= radius * radius
}

fn blend_pixel(destination: &mut Rgba<u8>, source: Rgba<u8>) {
    let alpha = source[3] as f32 / 255.0;
    for channel in 0..3 {
        destination[channel] = (source[channel] as f32 * alpha
            + destination[channel] as f32 * (1.0 - alpha))
            .round() as u8;
    }
    destination[3] = 255;
}

fn force_opaque(image: &mut RgbaImage) {
    for pixel in image.pixels_mut() {
        pixel[3] = 255;
    }
}

fn parse_color(value: &str) -> Result<Rgba<u8>> {
    anyhow::ensure!(
        value.len() == 7 && value.starts_with('#'),
        "invalid #RRGGBB color: {value}"
    );
    Ok(Rgba([
        u8::from_str_radix(&value[1..3], 16)?,
        u8::from_str_radix(&value[3..5], 16)?,
        u8::from_str_radix(&value[5..7], 16)?,
        255,
    ]))
}

fn lerp(start: u8, end: u8, t: f32) -> u8 {
    (start as f32 + (end as f32 - start as f32) * t).round() as u8
}

fn layout_from_id(value: &str) -> Result<Layout> {
    match value {
        "ui_dominant" => Ok(Layout::UiDominant),
        "device_bottom" => Ok(Layout::DeviceBottom),
        "device_center" => Ok(Layout::DeviceCenter),
        "ui_focus" => Ok(Layout::UiFocus),
        _ => anyhow::bail!("unsupported layout: {value}"),
    }
}

fn story_role_from_id(value: &str) -> Result<StoryRole> {
    match value {
        "hero" => Ok(StoryRole::Hero),
        "overview" => Ok(StoryRole::Overview),
        "detail" => Ok(StoryRole::Detail),
        "proof" => Ok(StoryRole::Proof),
        "synthesis" => Ok(StoryRole::Synthesis),
        _ => anyhow::bail!("unsupported story role: {value}"),
    }
}

fn composition_from_id(value: &str) -> Result<Composition> {
    match value {
        "editorial_hero" => Ok(Composition::EditorialHero),
        "editorial_split" => Ok(Composition::EditorialSplit),
        "chapter_field" => Ok(Composition::ChapterField),
        "synthesis_dark" => Ok(Composition::SynthesisDark),
        _ => anyhow::bail!("unsupported composition: {value}"),
    }
}

fn decoration_from_id(value: &str) -> Result<Decoration> {
    match value {
        "none" => Ok(Decoration::None),
        "spectrum" => Ok(Decoration::Spectrum),
        "orbit" => Ok(Decoration::Orbit),
        "grid" => Ok(Decoration::Grid),
        "signal" => Ok(Decoration::Signal),
        _ => anyhow::bail!("unsupported decoration: {value}"),
    }
}

fn list_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "none supplied".to_string()
    } else {
        values.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_color() {
        assert_eq!(parse_color("#64E4D2").unwrap(), Rgba([100, 228, 210, 255]));
        assert!(parse_color("64E4D2").is_err());
    }

    #[test]
    fn target_specific_raw_sources_override_primary_with_safe_fallback() {
        let catalog = RawSourceCatalog {
            primary_target: "phone".into(),
            by_target: BTreeMap::from([
                ("phone".into(), vec![PathBuf::from("phone/01.png")]),
                ("tablet".into(), vec![PathBuf::from("tablet/01.png")]),
            ]),
        };

        assert_eq!(
            catalog.for_target("tablet")[0],
            PathBuf::from("tablet/01.png")
        );
        assert_eq!(
            catalog.for_target("unknown")[0],
            PathBuf::from("phone/01.png")
        );
    }

    #[test]
    fn art_direction_recipes_create_story_rhythm() {
        let generation = Generation {
            brand_name: "Test".into(),
            tagline: String::new(),
            source_target: "phone".into(),
            frame_count: 5,
            generator_backend: "openrouter".into(),
            generator_model: "test".into(),
            style_direction: String::new(),
            palette: vec![
                "#000000".into(),
                "#111111".into(),
                "#5555FF".into(),
                "#FFFFFF".into(),
            ],
            allowed_layouts: vec!["ui_dominant".into()],
            creative_families: vec!["product_led".into()],
            art_direction: crate::spec::ArtDirection {
                story_roles: Vec::new(),
                allowed_compositions: vec![
                    "editorial_hero".into(),
                    "editorial_split".into(),
                    "chapter_field".into(),
                    "synthesis_dark".into(),
                ],
                allowed_decorations: vec![
                    "spectrum".into(),
                    "orbit".into(),
                    "grid".into(),
                    "signal".into(),
                ],
                frame_accents: vec!["#5555FF".into(), "#22AA77".into()],
                max_consecutive_same_composition: 2,
                min_unique_compositions: 3,
            },
            segments: Vec::new(),
            verified_claim_tokens: Vec::new(),
            store_tone_profiles: Default::default(),
        };

        let recipes = frame_recipes(&generation, 0).unwrap();
        assert_eq!(recipes[0].role, StoryRole::Hero);
        assert_eq!(recipes[1].role, StoryRole::Overview);
        assert_eq!(recipes[4].role, StoryRole::Synthesis);
        assert_eq!(recipes[0].composition, Composition::EditorialHero);
        assert_eq!(recipes[4].composition, Composition::SynthesisDark);
        assert!(
            recipes
                .iter()
                .map(|recipe| recipe.composition)
                .collect::<HashSet<_>>()
                .len()
                >= 3
        );
    }

    #[test]
    fn normalized_plans_cannot_escape_candidate_directory() {
        let generation = Generation {
            brand_name: "Test".into(),
            tagline: String::new(),
            source_target: "phone".into(),
            frame_count: 1,
            generator_backend: "openrouter".into(),
            generator_model: "test".into(),
            style_direction: String::new(),
            palette: vec![
                "#000000".into(),
                "#111111".into(),
                "#222222".into(),
                "#FFFFFF".into(),
            ],
            allowed_layouts: vec!["device_bottom".into()],
            creative_families: vec!["product_led".into()],
            art_direction: Default::default(),
            segments: Vec::new(),
            verified_claim_tokens: Vec::new(),
            store_tone_profiles: Default::default(),
        };
        let spec = test_spec();
        let segment = GenerationSegment {
            id: "default".into(),
            audience: "test audience".into(),
            intent: "test intent".into(),
            keywords: Vec::new(),
        };
        let mut plans = vec![CreativePlan {
            id: "../../escape".into(),
            family: CreativeFamily::Product,
            hypothesis_id: "unsafe".into(),
            hypothesis: "Showing real UI first will improve comprehension.".into(),
            concept: "truthful".into(),
            palette: PalettePlan {
                background_start: "#000000".into(),
                background_end: "#111111".into(),
                accent: "#222222".into(),
                text: "#FFFFFF".into(),
                muted: "#FFFFFF".into(),
            },
            frames: vec![FramePlan {
                index: 1,
                badge: "badge".into(),
                headline: "headline".into(),
                body: "body".into(),
                chips: vec![],
                layout: Layout::DeviceBottom,
                role: StoryRole::Legacy,
                composition: Composition::Legacy,
                decoration: Decoration::None,
                accent: None,
                footer: "payoff".into(),
            }],
            feature: FeaturePlan {
                headline: "headline".into(),
                body: "body".into(),
                chips: vec![],
                source_indices: vec![1],
            },
        }];
        normalize_and_validate(&mut plans, &spec, &generation, 1, 1, &segment).unwrap();
        assert_eq!(plans[0].id, "variant_01");
        assert_eq!(plans[0].hypothesis_id, "r01-default-01");
    }

    #[test]
    fn unverified_social_proof_is_rejected() {
        let generation = Generation {
            brand_name: "Test".into(),
            tagline: String::new(),
            source_target: "phone".into(),
            frame_count: 1,
            generator_backend: "openrouter".into(),
            generator_model: "test".into(),
            style_direction: String::new(),
            palette: vec![
                "#000000".into(),
                "#111111".into(),
                "#222222".into(),
                "#FFFFFF".into(),
            ],
            allowed_layouts: vec!["ui_dominant".into()],
            creative_families: vec!["product_led".into()],
            art_direction: Default::default(),
            segments: Vec::new(),
            verified_claim_tokens: Vec::new(),
            store_tone_profiles: Default::default(),
        };
        let spec = test_spec();
        let segment = GenerationSegment {
            id: "default".into(),
            audience: "test audience".into(),
            intent: "test intent".into(),
            keywords: Vec::new(),
        };
        let mut plans = vec![CreativePlan {
            id: "variant".into(),
            family: CreativeFamily::Product,
            hypothesis_id: "hypothesis".into(),
            hypothesis: "Real UI improves comprehension.".into(),
            concept: "truthful".into(),
            palette: PalettePlan {
                background_start: "#000000".into(),
                background_end: "#111111".into(),
                accent: "#222222".into(),
                text: "#FFFFFF".into(),
                muted: "#FFFFFF".into(),
            },
            frames: vec![FramePlan {
                index: 1,
                badge: "badge".into(),
                headline: "평점 4.9 리뷰 점수".into(),
                body: "body".into(),
                chips: vec![],
                layout: Layout::UiDominant,
                role: StoryRole::Legacy,
                composition: Composition::Legacy,
                decoration: Decoration::None,
                accent: None,
                footer: "payoff".into(),
            }],
            feature: FeaturePlan {
                headline: "headline".into(),
                body: "body".into(),
                chips: vec![],
                source_indices: vec![1],
            },
        }];

        let error =
            normalize_and_validate(&mut plans, &spec, &generation, 1, 1, &segment).unwrap_err();
        assert!(error.to_string().contains("unverified trust marker"));
    }

    fn test_spec() -> Spec {
        Spec {
            name: "test".into(),
            context: String::new(),
            audience: "test audience".into(),
            product_truth: Vec::new(),
            prohibited_claims: Vec::new(),
            thumbnail_width: 220,
            critic_backends: vec!["openrouter".into()],
            openrouter_critic_model: "test".into(),
            targets: Vec::new(),
            criteria: Vec::new(),
            lenses: Vec::new(),
            generation: None,
            experiment: Default::default(),
        }
    }
}
