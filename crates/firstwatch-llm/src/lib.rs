use anyhow::{bail, Context, Result};
use firstwatch_schema::{
    ChangeEvent, ChangeKind, ChangeSource, Confidence, Impact, SCHEMA_VERSION,
};
use serde_json::{json, Value};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const DEFAULT_MODEL: &str = "claude-opus-5";

pub struct Llm {
    api_key: String,
    model: String,
}

impl Llm {
    /// Reads ANTHROPIC_API_KEY and optional FIRSTWATCH_LLM_MODEL.
    /// None when no key is configured; the caller decides how to degrade.
    pub fn from_env() -> Option<Llm> {
        let api_key = std::env::var("ANTHROPIC_API_KEY").ok()?;
        if api_key.is_empty() {
            return None;
        }
        let model = std::env::var("FIRSTWATCH_LLM_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        Some(Llm { api_key, model })
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
        let body = json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "output_config": {"format": {"type": "json_schema", "schema": schema}},
            "messages": [{"role": "user", "content": prompt}],
        });
        let response: Value = ureq::post(API_URL)
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", "2023-06-01")
            .set("content-type", "application/json")
            .send_json(body)
            .map_err(|e| match e {
                ureq::Error::Status(code, resp) => {
                    let detail = resp.into_string().unwrap_or_default();
                    anyhow::anyhow!("claude api returned {code}: {detail}")
                }
                other => anyhow::anyhow!("claude api request failed: {other}"),
            })?
            .into_json()
            .context("parsing claude api response")?;

        if response["stop_reason"].as_str() == Some("refusal") {
            bail!("claude declined the request");
        }
        let text = response["content"]
            .as_array()
            .and_then(|blocks| {
                blocks
                    .iter()
                    .find(|b| b["type"].as_str() == Some("text"))
                    .and_then(|b| b["text"].as_str())
            })
            .context("no text block in claude response")?;
        serde_json::from_str(text).context("parsing structured output json")
    }
}
