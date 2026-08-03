// Vision-call adapter ported from Loop-Suite/icon-loop (Apache-2.0), then adapted
// for multi-target store creative contact sheets.
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::de::{DeserializeOwned, IntoDeserializer};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

#[derive(Clone, Debug)]
pub enum Provider {
    ClaudeCli { bin: String },
    OpenRouter { api_key: String },
}

#[derive(Clone, Debug)]
pub struct Llm {
    pub provider_label: &'static str,
    provider: Provider,
    model: Option<String>,
    retries: u32,
    verbose: bool,
}

impl Llm {
    pub fn claude_cli(bin: String, model: Option<String>, retries: u32, verbose: bool) -> Self {
        Self {
            provider_label: "claude",
            provider: Provider::ClaudeCli { bin },
            model,
            retries,
            verbose,
        }
    }

    pub fn openrouter(model: String, retries: u32, verbose: bool) -> Result<Self> {
        let api_key =
            std::env::var("OPENROUTER_API_KEY").context("OPENROUTER_API_KEY is not set")?;
        Ok(Self {
            provider_label: "openrouter",
            provider: Provider::OpenRouter { api_key },
            model: Some(model),
            retries,
            verbose,
        })
    }

    pub fn json_with_images<T: DeserializeOwned>(
        &self,
        prompt: &str,
        system: &str,
        images: &[PathBuf],
    ) -> Result<T> {
        let mut last_error = None;
        for attempt in 0..=self.retries {
            let result = self
                .call_once(prompt, Some(system), images)
                .and_then(|raw| extract_json(&raw))
                .and_then(|json| {
                    serde_path_to_error::deserialize(json.into_deserializer()).map_err(|error| {
                        anyhow!(
                            "response does not match {} at {}: {}",
                            std::any::type_name::<T>(),
                            error.path(),
                            error.inner()
                        )
                    })
                });
            match result {
                Ok(value) => return Ok(value),
                Err(error) => last_error = Some(error),
            }
            if self.verbose {
                eprintln!(
                    "[json attempt {}/{}] {:#}",
                    attempt + 1,
                    self.retries + 1,
                    last_error.as_ref().expect("retry error")
                );
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow!("LLM request failed")))
    }

    fn call_once(&self, prompt: &str, system: Option<&str>, images: &[PathBuf]) -> Result<String> {
        match &self.provider {
            Provider::ClaudeCli { bin } => {
                call_claude(bin, self.model.as_deref(), prompt, system, images)
            }
            Provider::OpenRouter { api_key } => {
                call_openrouter(api_key, self.model.as_deref(), prompt, system, images)
            }
        }
    }
}

fn call_claude(
    bin: &str,
    model: Option<&str>,
    prompt: &str,
    system: Option<&str>,
    images: &[PathBuf],
) -> Result<String> {
    let mut command = Command::new(bin);
    command
        .arg("-p")
        .arg("--output-format")
        .arg("json")
        .arg("--safe-mode")
        .arg("--disable-slash-commands")
        .arg("--no-session-persistence");
    if images.is_empty() {
        command.arg("--tools").arg("");
    } else {
        let common = common_ancestor(images).context("image paths have no common parent")?;
        command
            .arg("--tools")
            .arg("Read")
            .arg("--allowedTools")
            .arg("Read")
            .arg("--add-dir")
            .arg(common);
    }
    if let Some(model) = model {
        command.arg("--model").arg(model);
    }
    if let Some(system) = system {
        command.arg("--append-system-prompt").arg(system);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to execute `{bin}`; check installation and PATH"))?;
    child
        .stdin
        .as_mut()
        .context("failed to open Claude stdin")?
        .write_all(prompt.as_bytes())?;
    drop(child.stdin.take());
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(anyhow!(
            "Claude exited with {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let envelope: serde_json::Value = serde_json::from_str(stdout.trim()).with_context(|| {
        format!(
            "failed to parse Claude envelope: {}",
            truncate(&stdout, 400)
        )
    })?;
    if envelope
        .get("is_error")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return Err(anyhow!("Claude error response: {}", truncate(&stdout, 400)));
    }
    envelope
        .get("result")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .context("Claude response has no result field")
}

fn call_openrouter(
    api_key: &str,
    model: Option<&str>,
    prompt: &str,
    system: Option<&str>,
    images: &[PathBuf],
) -> Result<String> {
    let mut messages = Vec::new();
    if let Some(system) = system {
        messages.push(serde_json::json!({"role": "system", "content": system}));
    }
    let mut parts = vec![serde_json::json!({"type": "text", "text": prompt})];
    for path in images {
        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read image: {}", path.display()))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        parts.push(serde_json::json!({
            "type": "image_url",
            "image_url": {"url": format!("data:{};base64,{encoded}", image_mime(path))}
        }));
    }
    messages.push(serde_json::json!({"role": "user", "content": parts}));
    let response = ureq::post(OPENROUTER_URL)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .send_json(serde_json::json!({
            "model": model,
            "messages": messages,
            "max_tokens": 8192,
            "temperature": 0.2
        }));
    let response = match response {
        Ok(response) => response,
        Err(ureq::Error::Status(code, response)) => {
            let body = response.into_string().unwrap_or_default();
            return Err(anyhow!("OpenRouter HTTP {code}: {}", truncate(&body, 400)));
        }
        Err(error) => return Err(anyhow!("OpenRouter request failed: {error}")),
    };
    let value: serde_json::Value = response
        .into_json()
        .context("failed to parse OpenRouter JSON")?;
    value
        .pointer("/choices/0/message/content")
        .and_then(|content| content.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow!(
                "OpenRouter response has no content: {}",
                truncate(&value.to_string(), 400)
            )
        })
}

fn image_mime(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

fn common_ancestor(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut common = paths.first()?.parent()?.to_path_buf();
    while paths.iter().any(|path| !path.starts_with(&common)) {
        if !common.pop() {
            return None;
        }
    }
    Some(common)
}

pub fn extract_json(raw: &str) -> Result<serde_json::Value> {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    if let Some(start) = trimmed.find("```") {
        let after = trimmed[start + 3..]
            .strip_prefix("json")
            .unwrap_or(&trimmed[start + 3..]);
        if let Some(end) = after.find("```") {
            if let Ok(value) = serde_json::from_str(after[..end].trim()) {
                return Ok(value);
            }
        }
    }
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            if let Ok(value) = serde_json::from_str(&trimmed[start..=end]) {
                return Ok(value);
            }
        }
    }
    Err(anyhow!(
        "failed to extract JSON: {}",
        truncate(trimmed, 400)
    ))
}

fn truncate(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        value.to_string()
    } else {
        value.chars().take(limit).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_fenced_model_output() {
        let value = extract_json("prefix\n```json\n{\"ok\":true}\n```\nsuffix").unwrap();
        assert_eq!(value["ok"], true);
    }
}
