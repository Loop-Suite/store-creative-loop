use crate::models::Candidate;
use crate::spec::Spec;
use anyhow::{Context, Result};
use image::{imageops::FilterType, DynamicImage, GenericImage, Rgba, RgbaImage};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const COLUMNS: u32 = 4;
const GAP: u32 = 24;

pub type BlindMap = BTreeMap<String, String>;
pub type ContactSheets = BTreeMap<String, Vec<PathBuf>>;

pub fn build_contact_sheets(
    candidates_root: &Path,
    candidates: &[Candidate],
    spec: &Spec,
    out: &Path,
) -> Result<(BlindMap, ContactSheets)> {
    let mut blind_map = BTreeMap::new();
    let mut sheets = BTreeMap::new();

    for (index, candidate) in candidates.iter().enumerate() {
        let blind_id = format!("candidate_{:02}", index + 1);
        blind_map.insert(blind_id.clone(), candidate.id.clone());
        let blind_dir = out.join("blind").join(&blind_id);
        std::fs::create_dir_all(&blind_dir)?;
        let mut paths = Vec::new();

        for target_assets in &candidate.targets {
            if target_assets.assets.is_empty() {
                continue;
            }
            let source_paths = target_assets
                .assets
                .iter()
                .map(|asset| candidates_root.join(&candidate.id).join(&asset.path))
                .collect::<Vec<_>>();
            let destination = blind_dir.join(format!("{}.png", target_assets.target_id));
            render_sheet(&source_paths, spec.thumbnail_width, &destination)?;
            paths.push(destination);
        }
        sheets.insert(blind_id, paths);
    }
    Ok((blind_map, sheets))
}

fn render_sheet(sources: &[PathBuf], cell_width: u32, destination: &Path) -> Result<()> {
    let mut images = Vec::new();
    let mut max_height = 1;
    for source in sources {
        let image = image::open(source).with_context(|| {
            format!("failed to open contact-sheet source: {}", source.display())
        })?;
        let height = ((image.height() as f64 * cell_width as f64 / image.width() as f64).round()
            as u32)
            .max(1);
        max_height = max_height.max(height);
        images.push(image.resize_exact(cell_width, height, FilterType::Lanczos3));
    }
    let rows = (images.len() as u32).div_ceil(COLUMNS);
    let width = GAP + COLUMNS * (cell_width + GAP);
    let height = GAP + rows * (max_height + GAP + 8);
    let mut canvas = RgbaImage::from_pixel(width, height, Rgba([245, 246, 248, 255]));

    for (index, image) in images.iter().enumerate() {
        let column = index as u32 % COLUMNS;
        let row = index as u32 / COLUMNS;
        let x = GAP + column * (cell_width + GAP);
        let y = GAP + row * (max_height + GAP + 8);
        let bar_color = Rgba([42, 47, 61, 255]);
        for bar_y in y.saturating_sub(6)..y.saturating_sub(2) {
            for bar_x in x..x + cell_width {
                canvas.put_pixel(bar_x, bar_y, bar_color);
            }
        }
        canvas.copy_from(&image.to_rgba8(), x, y)?;
    }
    DynamicImage::ImageRgba8(canvas)
        .save(destination)
        .with_context(|| format!("failed to save contact sheet: {}", destination.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn contact_sheet_is_created_at_controlled_thumbnail_width() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("01.png");
        let second = temp.path().join("02.png");
        RgbaImage::from_pixel(40, 80, Rgba([10, 20, 30, 255]))
            .save(&first)
            .unwrap();
        RgbaImage::from_pixel(40, 80, Rgba([30, 20, 10, 255]))
            .save(&second)
            .unwrap();
        let out = temp.path().join("sheet.png");
        render_sheet(&[first, second], 80, &out).unwrap();
        let sheet = image::open(out).unwrap();
        assert_eq!(sheet.width(), GAP + COLUMNS * (80 + GAP));
        assert!(sheet.height() > 160);
    }
}
