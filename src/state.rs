use crate::models::State;
use anyhow::{Context, Result};
use std::path::Path;

pub fn write(path: &Path, state: &State) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(state)?;
    std::fs::write(path, bytes)
        .with_context(|| format!("failed to write state: {}", path.display()))
}

pub fn load(path: &Path) -> Result<State> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read prior state: {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse prior state: {}", path.display()))
}
