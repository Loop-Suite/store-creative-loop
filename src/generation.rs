use crate::llm::Llm;
use crate::spec::{AssetKind, Generation, Spec, Target};
use ab_glyph::{FontArc, PxScale};
use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::{Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_filled_rect_mut, draw_text_mut, text_size};
use imageproc::rect::Rect;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const PLAN_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    DeviceBottom,
    DeviceCenter,
    UiFocus,
}

impl Layout {
    fn id(self) -> &'static str {
        match self {
            Self::DeviceBottom => "device_bottom",
            Self::DeviceCenter => "device_center",
            Self::UiFocus => "ui_focus",
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
    pub raw_sources: Vec<String>,
    pub plans: Vec<CreativePlan>,
}

pub fn discover_raw_sources(raw_root: &Path, generation: &Generation) -> Result<Vec<PathBuf>> {
    let source_dir = raw_root.join(&generation.source_target);
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
        sources.len() >= generation.frame_count,
        "generation needs {} raw images in {}, found {}",
        generation.frame_count,
        source_dir.display(),
        sources.len()
    );
    sources.truncate(generation.frame_count);
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
    feedback: Option<&str>,
) -> Result<Vec<CreativePlan>> {
    let template = plan_template(generation, variants)?;
    let source_catalog = sources
        .iter()
        .enumerate()
        .map(|(index, path)| format!("- frame {}: {}", index + 1, path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "# Product\nContext: {}\nAudience: {}\nProduct truths: {}\nProhibited claims: {}\n\n\
         # Store-creative generation task\nCreate exactly {variants} materially different, production-ready creative plans for round {round}. Each plan turns the same ordered raw app captures into a coherent screenshot story. The deterministic renderer—not you—will draw the final pixels.\n\n\
         Brand: {}\nTagline: {}\nStyle direction: {}\nAllowed palette values (use these exact strings only): {}\nAllowed layouts: {}\nRequired screenshot frames per plan: {}\n\n\
         # Ordered raw captures\n{}\nThe attached image is their contact sheet in the same left-to-right, top-to-bottom order. Do not invent product UI.\n\n\
         # Prior-round feedback\n{}\n\n\
         Write concise store copy, not captions that merely describe the pixels. Headline: at most 24 Korean characters or 42 Latin characters and at most two visual lines. Body: one short sentence. Use 0–3 short chips. Make the first three frames carry the product promise, proof, and differentiation. Later frames should add non-redundant evidence. Variants must differ in message hierarchy and visual rhythm, not in product truth. Feature art must work at 1024x500 and may use one or two valid source indices.\n\n\
         Return JSON only. Keep every id/index/field in this complete template, replace all placeholder copy, and preserve the exact number of plans and frames:\n{}",
        spec.context,
        spec.audience,
        list_or_none(&spec.product_truth),
        list_or_none(&spec.prohibited_claims),
        generation.brand_name,
        generation.tagline,
        generation.style_direction,
        generation.palette.join(", "),
        generation.allowed_layouts.join(", "),
        generation.frame_count,
        source_catalog,
        feedback.unwrap_or("No prior round. Explore distinct evidence-backed directions."),
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
            normalize_and_validate(&mut response.plans, generation, variants)?;
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
    sources: &[PathBuf],
    plans: &[CreativePlan],
) -> Result<()> {
    let manifest = GenerationManifest {
        round,
        generator_provider: llm.provider_label.to_string(),
        raw_sources: sources
            .iter()
            .map(|path| path.display().to_string())
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
    sources: &[PathBuf],
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
            let target_dir = candidate_dir.join(&target.id);
            std::fs::create_dir_all(&target_dir)?;
            let (width, height) = target_size(target)?;
            match target.kind {
                AssetKind::Screenshot => {
                    for frame in &plan.frames {
                        let source = image::open(&sources[frame.index - 1]).with_context(|| {
                            format!(
                                "failed to open raw screenshot: {}",
                                sources[frame.index - 1].display()
                            )
                        })?;
                        let mut rendered = render_screenshot(
                            width,
                            height,
                            &generation.brand_name,
                            frame,
                            &plan.palette,
                            &font,
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
                        sources,
                    )?;
                    force_opaque(&mut rendered);
                    rendered.save(target_dir.join("01.png"))?;
                }
            }
        }
    }
    Ok(())
}

fn plan_template(generation: &Generation, variants: usize) -> Result<String> {
    let layouts = generation
        .allowed_layouts
        .iter()
        .map(|layout| layout_from_id(layout))
        .collect::<Result<Vec<_>>>()?;
    let plans = (0..variants)
        .map(|variant| {
            let frames = (0..generation.frame_count)
                .map(|index| {
                    serde_json::json!({
                        "index": index + 1,
                        "badge": "replace",
                        "headline": "replace",
                        "body": "replace",
                        "chips": ["replace"],
                        "layout": layouts[(index + variant) % layouts.len()].id()
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "id": format!("variant_{:02}", variant + 1),
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
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::to_string_pretty(
        &serde_json::json!({"plans": plans}),
    )?)
}

fn normalize_and_validate(
    plans: &mut [CreativePlan],
    generation: &Generation,
    variants: usize,
) -> Result<()> {
    anyhow::ensure!(
        plans.len() == variants,
        "expected {variants} plans, found {}",
        plans.len()
    );
    let allowed_colors = generation.palette.iter().cloned().collect::<HashSet<_>>();
    let allowed_layouts = generation
        .allowed_layouts
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    for (plan_index, plan) in plans.iter_mut().enumerate() {
        plan.id = format!("variant_{:02}", plan_index + 1);
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
        for (expected_index, frame) in plan.frames.iter().enumerate() {
            anyhow::ensure!(
                frame.index == expected_index + 1,
                "{} frame indices must be exactly 1..{}",
                plan.id,
                generation.frame_count
            );
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
    }
    Ok(())
}

fn render_screenshot(
    width: u32,
    height: u32,
    brand: &str,
    frame: &FramePlan,
    palette: &PalettePlan,
    font: &FontArc,
    source: &RgbaImage,
) -> Result<RgbaImage> {
    let start = parse_color(&palette.background_start)?;
    let end = parse_color(&palette.background_end)?;
    let accent = parse_color(&palette.accent)?;
    let text = parse_color(&palette.text)?;
    let muted = parse_color(&palette.muted)?;
    let mut canvas = gradient(width, height, start, end);
    add_decorations(&mut canvas, accent);
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
        Layout::DeviceBottom => (height as f32 * 0.37) as i32,
        Layout::DeviceCenter => (height as f32 * 0.33) as i32,
        Layout::UiFocus => (height as f32 * 0.31) as i32,
    };
    let max_width = (width as f32
        * match frame.layout {
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
    );
    Ok(canvas)
}

fn render_feature(
    width: u32,
    height: u32,
    brand: &str,
    feature: &FeaturePlan,
    palette: &PalettePlan,
    font: &FontArc,
    sources: &[PathBuf],
) -> Result<RgbaImage> {
    let start = parse_color(&palette.background_start)?;
    let end = parse_color(&palette.background_end)?;
    let accent = parse_color(&palette.accent)?;
    let text = parse_color(&palette.text)?;
    let muted = parse_color(&palette.muted)?;
    let mut canvas = gradient(width, height, start, end);
    add_decorations(&mut canvas, accent);
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
        Rgba([0, 0, 0, 100]),
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
    let line_width = 2_i32;
    for offset in 0..line_width {
        let color = Rgba([accent[0], accent[1], accent[2], 120]);
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

fn add_decorations(canvas: &mut RgbaImage, accent: Rgba<u8>) {
    let width = canvas.width() as i32;
    let height = canvas.height() as i32;
    for index in 0..5 {
        let radius = (width as f32 * (0.16 + index as f32 * 0.035)) as i32;
        let color = Rgba([
            accent[0],
            accent[1],
            accent[2],
            12_u8.saturating_sub(index * 2),
        ]);
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
    draw_filled_rect_mut(
        canvas,
        Rect::at((width as f32 * 0.075) as i32, (height as f32 * 0.12) as i32)
            .of_size(bar_width, bar_height),
        accent,
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
        "device_bottom" => Ok(Layout::DeviceBottom),
        "device_center" => Ok(Layout::DeviceCenter),
        "ui_focus" => Ok(Layout::UiFocus),
        _ => anyhow::bail!("unsupported layout: {value}"),
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
        };
        let mut plans = vec![CreativePlan {
            id: "../../escape".into(),
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
            }],
            feature: FeaturePlan {
                headline: "headline".into(),
                body: "body".into(),
                chips: vec![],
                source_indices: vec![1],
            },
        }];
        normalize_and_validate(&mut plans, &generation, 1).unwrap();
        assert_eq!(plans[0].id, "variant_01");
    }
}
