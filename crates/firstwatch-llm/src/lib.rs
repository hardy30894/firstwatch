use anyhow::{bail, Context, Result};
use firstwatch_schema::{
    ChangeEvent, ChangeKind, ChangeSource, Confidence, Impact, SCHEMA_VERSION,
};
use serde_json::{json, Value};

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_DEFAULT_MODEL: &str = "claude-opus-5";

/// Two wire formats cover the hosted ecosystem: Anthropic's, and the
/// OpenAI-compatible format spoken by Ollama, vLLM, Groq, OpenRouter,
/// and most gateways. Command is the escape hatch for everything else:
/// any executable that reads a prompt on stdin and writes json to stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Wire {
    Anthropic,
    OpenAiCompatible,
    Command(String),
}

pub struct Llm {
    wire: Wire,
    base_url: String,
    api_key: String,
    model: String,
}

impl Llm {
    /// Provider selection, all via env:
    /// - FIRSTWATCH_LLM_PROVIDER: `anthropic` (default) or `openai`
    ///   (any OpenAI-compatible server: Ollama, vLLM, OpenRouter, ...)
    /// - FIRSTWATCH_LLM_CMD: escape hatch for custom or internal APIs.
    ///   A shell command that reads the prompt on stdin and writes json
    ///   matching the schema (given as $FIRSTWATCH_LLM_SCHEMA) to stdout.
    ///   Takes precedence over FIRSTWATCH_LLM_PROVIDER.
    /// - FIRSTWATCH_LLM_BASE_URL: endpoint override
    /// - FIRSTWATCH_LLM_MODEL: model override
    /// - FIRSTWATCH_LLM_API_KEY, else ANTHROPIC_API_KEY or OPENAI_API_KEY
    ///   per provider. Local servers may not need one; `none` disables auth.
    ///
    /// None when no usable configuration exists; the caller decides how
    /// to degrade.
    pub fn from_env() -> Option<Llm> {
        if let Ok(cmd) = std::env::var("FIRSTWATCH_LLM_CMD") {
            if !cmd.is_empty() {
                return Some(Llm {
                    wire: Wire::Command(cmd),
                    base_url: String::new(),
                    api_key: String::new(),
                    model: String::new(),
                });
            }
        }
        let provider = std::env::var("FIRSTWATCH_LLM_PROVIDER")
            .unwrap_or_else(|_| "anthropic".into())
            .to_lowercase();
        let (wire, default_url, default_model, key_var) = match provider.as_str() {
            "openai" | "openai-compatible" => (
                Wire::OpenAiCompatible,
                "https://api.openai.com/v1/chat/completions",
                "",
                "OPENAI_API_KEY",
            ),
            _ => (
                Wire::Anthropic,
                ANTHROPIC_URL,
                ANTHROPIC_DEFAULT_MODEL,
                "ANTHROPIC_API_KEY",
            ),
        };
        let api_key = std::env::var("FIRSTWATCH_LLM_API_KEY")
            .or_else(|_| std::env::var(key_var))
            .ok()?;
        if api_key.is_empty() {
            return None;
        }
        let base_url =
            std::env::var("FIRSTWATCH_LLM_BASE_URL").unwrap_or_else(|_| default_url.into());
        let model = std::env::var("FIRSTWATCH_LLM_MODEL").unwrap_or_else(|_| default_model.into());
        if model.is_empty() {
            return None;
        }
        Some(Llm {
            wire,
            base_url,
            api_key,
            model,
        })
    }

    /// Synthesizes a replacement for one impacted line. The result is a
    /// proposal only; the prove gate decides whether it ships.
    pub fn fix_line(&self, impact: &Impact, old_line: &str, context: &str) -> Result<String> {
        let change = &impact.change;
        let prompt = format!(
            "An upstream API changed and one line of code is affected.\n\n\
             Provider: {}\nSurface: {}\nChange kind: {:?}\nField: {}\nFrom: {}\nTo: {}\n\n\
             Surrounding code for context:\n```\n{}\n```\n\n\
             The affected line is:\n```\n{}\n```\n\n\
             Rewrite only that line so the code works with the changed API. \
             Preserve indentation and style. If the change cannot be expressed \
             as a single-line rewrite, set fixable to false.",
            change.provider,
            change.surface,
            change.kind,
            change.field.as_deref().unwrap_or("(operation level)"),
            change.from.as_deref().unwrap_or("-"),
            change.to.as_deref().unwrap_or("-"),
            context,
            old_line,
        );
        let schema = json!({
            "type": "object",
            "properties": {
                "fixable": {"type": "boolean"},
                "new_line": {"type": "string"}
            },
            "required": ["fixable", "new_line"],
            "additionalProperties": false
        });
        let output = self.request(&prompt, 4096, &schema)?;
        if !output["fixable"].as_bool().unwrap_or(false) {
            bail!("model judged the change not fixable as a single-line rewrite");
        }
        let new_line = output["new_line"]
            .as_str()
            .context("model output missing new_line")?;
        if new_line.trim().is_empty() || new_line == old_line {
            bail!("model returned an empty or unchanged line");
        }
        if new_line.contains('\n') {
            bail!("model returned a multi-line rewrite for a single-line patch");
        }
        Ok(new_line.to_string())
    }

    /// Parses prose release notes into ChangeEvents. Everything extracted
    /// is flagged low confidence; spec diffs remain the high-confidence path.
    pub fn parse_changelog(&self, text: &str, provider: &str) -> Result<Vec<ChangeEvent>> {
        let prompt = format!(
            "Extract API changes from these release notes for the provider `{provider}`.\n\n\
             Report only concrete changes to API operations or their parameters: \
             removals, renames, type changes, newly required parameters, \
             deprecations, and new operations. Skip marketing, bug fixes, and \
             internal changes.\n\n\
             `surface` is the SDK-style path, like `charges.create` or \
             `chat.completions.create`. `field` is the parameter name when the \
             change is to a parameter, else null. `to` is the replacement \
             surface or field for renames and deprecations, else null.\n\n\
             Release notes:\n---\n{text}\n---"
        );
        let schema = json!({
            "type": "object",
            "properties": {
                "changes": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "surface": {"type": "string"},
                            "kind": {"type": "string", "enum": [
                                "removed", "renamed", "type_changed",
                                "required_added", "deprecated", "added"
                            ]},
                            "field": {"type": ["string", "null"]},
                            "to": {"type": ["string", "null"]},
                            "effective_date": {"type": ["string", "null"]}
                        },
                        "required": ["surface", "kind", "field", "to", "effective_date"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["changes"],
            "additionalProperties": false
        });
        let output = self.request(&prompt, 16000, &schema)?;
        let changes = output["changes"]
            .as_array()
            .context("model output missing changes array")?;
        let mut events = Vec::new();
        for change in changes {
            let kind = match change["kind"].as_str() {
                Some("removed") => ChangeKind::Removed,
                Some("renamed") => ChangeKind::Renamed,
                Some("type_changed") => ChangeKind::TypeChanged,
                Some("required_added") => ChangeKind::RequiredAdded,
                Some("deprecated") => ChangeKind::Deprecated,
                Some("added") => ChangeKind::Added,
                _ => continue,
            };
            let Some(surface) = change["surface"].as_str() else {
                continue;
            };
            events.push(ChangeEvent {
                schema_version: SCHEMA_VERSION.into(),
                provider: provider.into(),
                surface: surface.into(),
                field: change["field"].as_str().map(String::from),
                kind,
                from: None,
                to: change["to"].as_str().map(String::from),
                effective_version: None,
                effective_date: change["effective_date"].as_str().map(String::from),
                confidence: Confidence::Low,
                source: ChangeSource::Changelog,
                evidence_url: None,
            });
        }
        Ok(events)
    }

    fn request(&self, prompt: &str, max_tokens: u32, schema: &Value) -> Result<Value> {
        let text = match &self.wire {
            Wire::Anthropic => self.request_anthropic(prompt, max_tokens, schema)?,
            Wire::OpenAiCompatible => self.request_openai(prompt, max_tokens, schema)?,
            Wire::Command(cmd) => request_command(cmd, prompt, schema)?,
        };
        parse_json_output(&text)
    }

    fn request_anthropic(&self, prompt: &str, max_tokens: u32, schema: &Value) -> Result<String> {
        let body = json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "output_config": {"format": {"type": "json_schema", "schema": schema}},
            "messages": [{"role": "user", "content": prompt}],
        });
        let response: Value = ureq::post(&self.base_url)
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", "2023-06-01")
            .set("content-type", "application/json")
            .send_json(body)
            .map_err(request_error)?
            .into_json()
            .context("parsing llm response")?;

        if response["stop_reason"].as_str() == Some("refusal") {
            bail!("the model declined the request");
        }
        response["content"]
            .as_array()
            .and_then(|blocks| {
                blocks
                    .iter()
                    .find(|b| b["type"].as_str() == Some("text"))
                    .and_then(|b| b["text"].as_str())
            })
            .map(String::from)
            .context("no text block in llm response")
    }

    fn request_openai(&self, prompt: &str, max_tokens: u32, schema: &Value) -> Result<String> {
        let prompt = format!(
            "{prompt}

Respond with only a JSON object matching this schema,              no prose and no code fences:
{schema}"
        );
        let body = json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "response_format": {"type": "json_object"},
            "messages": [{"role": "user", "content": prompt}],
        });
        let mut request = ureq::post(&self.base_url).set("content-type", "application/json");
        if self.api_key != "none" {
            request = request.set("authorization", &format!("Bearer {}", self.api_key));
        }
        let response: Value = request
            .send_json(body)
            .map_err(request_error)?
            .into_json()
            .context("parsing llm response")?;
        response["choices"][0]["message"]["content"]
            .as_str()
            .map(String::from)
            .context("no message content in llm response")
    }
}

fn request_command(cmd: &str, prompt: &str, schema: &Value) -> Result<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("sh")
        .args(["-c", cmd])
        .env("FIRSTWATCH_LLM_SCHEMA", schema.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning llm command: {cmd}"))?;
    child
        .stdin
        .take()
        .context("opening llm command stdin")?
        .write_all(prompt.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "llm command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn request_error(e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Status(code, resp) => {
            let detail = resp.into_string().unwrap_or_default();
            anyhow::anyhow!("llm api returned {code}: {detail}")
        }
        other => anyhow::anyhow!("llm api request failed: {other}"),
    }
}

/// Tolerates code fences and surrounding prose from servers that ignore
/// the json-only instruction.
fn parse_json_output(text: &str) -> Result<Value> {
    let trimmed = text.trim();
    if let Ok(v) = serde_json::from_str(trimmed) {
        return Ok(v);
    }
    let inner = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if let Ok(v) = serde_json::from_str(inner) {
        return Ok(v);
    }
    let (start, end) = (trimmed.find('{'), trimmed.rfind('}'));
    if let (Some(start), Some(end)) = (start, end) {
        if start < end {
            return serde_json::from_str(&trimmed[start..=end])
                .context("parsing json embedded in llm output");
        }
    }
    bail!("llm output is not json: {trimmed}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clean_fenced_and_embedded_json() {
        assert!(parse_json_output(r#"{"a": 1}"#).is_ok());
        assert!(parse_json_output("```json\n{\"a\": 1}\n```").is_ok());
        assert!(parse_json_output(r#"Here you go: {"a": 1} hope that helps"#).is_ok());
        assert!(parse_json_output("no json here").is_err());
    }
}
