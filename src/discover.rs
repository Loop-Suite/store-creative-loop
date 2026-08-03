use crate::models::{AssetInfo, Candidate, PolicyIssue, Severity, TargetAssets};
use crate::spec::{AssetKind, Spec, Store, Target};
use anyhow::{Context, Result};
use image::ImageReader;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub fn discover_candidates(root: &Path, spec: &Spec) -> Result<Vec<Candidate>> {
    let mut dirs = std::fs::read_dir(root)
        .with_context(|| format!("failed to read candidates directory: {}", root.display()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    dirs.sort();
    anyhow::ensure!(
        !dirs.is_empty(),
        "no candidate directories found in {}",
        root.display()
    );

    dirs.into_iter()
        .map(|dir| inspect_candidate(&dir, spec))
        .collect()
}

fn inspect_candidate(dir: &Path, spec: &Spec) -> Result<Candidate> {
    let id = dir
        .file_name()
        .and_then(|name| name.to_str())
        .context("candidate directory name is not valid UTF-8")?
        .to_string();
    let mut targets = Vec::new();
    let mut issues = Vec::new();

    for target in &spec.targets {
        let target_dir = dir.join(&target.id);
        let mut files = image_files(&target_dir)?;
        files.sort();
        let mut assets = Vec::new();

        for file in &files {
            match inspect_image(file, dir) {
                Ok(asset) => {
                    validate_asset(target, &asset, &mut issues);
                    assets.push(asset);
                }
                Err(error) => issues.push(PolicyIssue {
                    target_id: target.id.clone(),
                    file: Some(relative_display(file, dir)),
                    code: "invalid_image".into(),
                    severity: Severity::Block,
                    evidence: error.to_string(),
                }),
            }
        }
        validate_count(target, assets.len(), &mut issues);
        validate_names(target, &files, &mut issues);
        validate_duplicates(target, &assets, &mut issues);
        targets.push(TargetAssets {
            target_id: target.id.clone(),
            assets,
        });
    }

    let hard_pass = !issues.iter().any(|issue| issue.severity == Severity::Block);
    Ok(Candidate {
        id,
        targets,
        policy_issues: issues,
        hard_pass,
    })
}

fn image_files(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("failed to read target directory: {}", dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let extension = entry
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp") {
            files.push(entry.path());
        }
    }
    Ok(files)
}

fn inspect_image(path: &Path, candidate_dir: &Path) -> Result<AssetInfo> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read image: {}", path.display()))?;
    let decoded = ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()?
        .decode()
        .with_context(|| format!("failed to decode image: {}", path.display()))?;
    let rgba = decoded.to_rgba8();
    let has_transparency = rgba.pixels().any(|pixel| pixel[3] != 255);
    Ok(AssetInfo {
        path: relative_display(path, candidate_dir),
        width: rgba.width(),
        height: rgba.height(),
        has_transparency,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    })
}

fn validate_asset(target: &Target, asset: &AssetInfo, issues: &mut Vec<PolicyIssue>) {
    let file = Some(asset.path.clone());
    if asset.has_transparency {
        issues.push(PolicyIssue {
            target_id: target.id.clone(),
            file: file.clone(),
            code: "transparency".into(),
            severity: Severity::Block,
            evidence: "store creative contains at least one non-opaque pixel".into(),
        });
    }

    if !target.allowed_sizes.is_empty()
        && !target
            .allowed_sizes
            .iter()
            .any(|size| size.width == asset.width && size.height == asset.height)
    {
        let allowed = target
            .allowed_sizes
            .iter()
            .map(|size| format!("{}x{}", size.width, size.height))
            .collect::<Vec<_>>()
            .join(", ");
        issues.push(PolicyIssue {
            target_id: target.id.clone(),
            file: file.clone(),
            code: "wrong_dimensions".into(),
            severity: Severity::Block,
            evidence: format!("found {}x{}; allowed: {allowed}", asset.width, asset.height),
        });
    }

    if target.store == Store::Google
        && target.kind == AssetKind::Screenshot
        && target.allowed_sizes.is_empty()
    {
        let shortest = asset.width.min(asset.height);
        let longest = asset.width.max(asset.height);
        if shortest < 320 || longest > 3840 || longest > shortest.saturating_mul(2) {
            issues.push(PolicyIssue {
                target_id: target.id.clone(),
                file: file.clone(),
                code: "google_screenshot_geometry".into(),
                severity: Severity::Block,
                evidence: format!(
                    "{}x{} violates Google Play's 320–3840 px and 2:1 maximum aspect constraints",
                    asset.width, asset.height
                ),
            });
        }
    }

    if target.store == Store::Google
        && target.kind == AssetKind::FeatureGraphic
        && (asset.width != 1024 || asset.height != 500)
    {
        issues.push(PolicyIssue {
            target_id: target.id.clone(),
            file,
            code: "google_feature_graphic_dimensions".into(),
            severity: Severity::Block,
            evidence: format!(
                "found {}x{}; Google feature graphics must be 1024x500",
                asset.width, asset.height
            ),
        });
    }
}

fn validate_count(target: &Target, count: usize, issues: &mut Vec<PolicyIssue>) {
    let block = |code: &str, evidence: String, issues: &mut Vec<PolicyIssue>| {
        issues.push(PolicyIssue {
            target_id: target.id.clone(),
            file: None,
            code: code.into(),
            severity: Severity::Block,
            evidence,
        });
    };
    if target.required && count == 0 {
        block(
            "missing_target",
            "required target has no image assets".into(),
            issues,
        );
        return;
    }
    if let Some(exact) = target.exact_assets {
        if count != exact {
            block(
                "wrong_asset_count",
                format!("found {count}; expected exactly {exact}"),
                issues,
            );
        }
    } else {
        if count > 0 && count < target.min_assets {
            block(
                "too_few_assets",
                format!("found {count}; minimum is {}", target.min_assets),
                issues,
            );
        }
        if let Some(max) = target.max_assets {
            if count > max {
                block(
                    "too_many_assets",
                    format!("found {count}; maximum is {max}"),
                    issues,
                );
            }
        }
    }
    let platform_max = match (target.store, target.kind) {
        (Store::Apple, AssetKind::Screenshot) => Some(10),
        (Store::Google, AssetKind::Screenshot) => Some(8),
        _ => None,
    };
    if let Some(max) = platform_max {
        if count > max {
            block(
                "platform_asset_limit",
                format!("found {count}; platform maximum is {max}"),
                issues,
            );
        }
    }
}

fn validate_names(target: &Target, files: &[PathBuf], issues: &mut Vec<PolicyIssue>) {
    if files.len() < 10 {
        return;
    }
    for file in files {
        let stem = file
            .file_stem()
            .and_then(|x| x.to_str())
            .unwrap_or_default();
        let leading_digits = stem
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        if leading_digits.len() == 1 {
            issues.push(PolicyIssue {
                target_id: target.id.clone(),
                file: file
                    .file_name()
                    .and_then(|x| x.to_str())
                    .map(str::to_string),
                code: "non_zero_padded_sequence".into(),
                severity: Severity::Warn,
                evidence: "lexical order may differ from intended frame order; prefer 01, 02, …"
                    .into(),
            });
        }
    }
}

fn validate_duplicates(target: &Target, assets: &[AssetInfo], issues: &mut Vec<PolicyIssue>) {
    let mut by_hash: HashMap<&str, Vec<&str>> = HashMap::new();
    for asset in assets {
        by_hash.entry(&asset.sha256).or_default().push(&asset.path);
    }
    let duplicates = by_hash
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(_, paths)| paths.join(", "))
        .collect::<Vec<_>>();
    if !duplicates.is_empty() {
        issues.push(PolicyIssue {
            target_id: target.id.clone(),
            file: None,
            code: "duplicate_assets".into(),
            severity: Severity::Warn,
            evidence: format!("byte-identical frames: {}", duplicates.join("; ")),
        });
    }
}

fn relative_display(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

pub fn issue_counts(candidate: &Candidate) -> BTreeMap<Severity, usize> {
    let mut counts = BTreeMap::new();
    for issue in &candidate.policy_issues {
        *counts.entry(issue.severity).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Criterion, Experiment, Lens, Size};
    use image::{Rgba, RgbaImage};
    use std::collections::BTreeSet;

    fn spec() -> Spec {
        Spec {
            name: "test".into(),
            context: String::new(),
            audience: String::new(),
            product_truth: Vec::new(),
            prohibited_claims: Vec::new(),
            thumbnail_width: 80,
            critic_backends: vec!["claude".into()],
            openrouter_critic_model: "test".into(),
            targets: vec![Target {
                id: "phone".into(),
                store: Store::Apple,
                device: "phone".into(),
                locale: "en".into(),
                kind: AssetKind::Screenshot,
                required: true,
                min_assets: 1,
                max_assets: Some(1),
                exact_assets: Some(1),
                allowed_sizes: vec![Size {
                    width: 10,
                    height: 20,
                }],
            }],
            criteria: vec![Criterion {
                id: "clarity".into(),
                name: "Clarity".into(),
                weight: 1.0,
                guide: "clear".into(),
            }],
            lenses: vec![Lens {
                id: "user".into(),
                name: "User".into(),
                instruction: "inspect".into(),
            }],
            generation: None,
            experiment: Experiment::default(),
        }
    }

    #[test]
    fn opaque_exact_image_passes_and_transparency_or_size_blocks() {
        let temp = tempfile::tempdir().unwrap();
        let good_dir = temp.path().join("good").join("phone");
        std::fs::create_dir_all(&good_dir).unwrap();
        RgbaImage::from_pixel(10, 20, Rgba([1, 2, 3, 255]))
            .save(good_dir.join("01.png"))
            .unwrap();
        let good = inspect_candidate(good_dir.parent().unwrap(), &spec()).unwrap();
        assert!(good.hard_pass);

        let bad_dir = temp.path().join("bad").join("phone");
        std::fs::create_dir_all(&bad_dir).unwrap();
        RgbaImage::from_pixel(11, 20, Rgba([1, 2, 3, 100]))
            .save(bad_dir.join("01.png"))
            .unwrap();
        let bad = inspect_candidate(bad_dir.parent().unwrap(), &spec()).unwrap();
        assert!(!bad.hard_pass);
        let codes = bad
            .policy_issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect::<BTreeSet<_>>();
        assert!(codes.contains("transparency"));
        assert!(codes.contains("wrong_dimensions"));
    }
}
