use std::io::{BufRead, Write};

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: &str = "0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Removed,
    Renamed,
    TypeChanged,
    RequiredAdded,
    Deprecated,
    Added,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSource {
    OpenapiDiff,
    SdkSurfaceDiff,
    Changelog,
    DeprecationHeader,
    Manual,
}

/// One upstream API change. The atom of the system.
/// `surface` is a provider-scoped path like `charges.create` or `POST /v1/charges`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEvent {
    pub schema_version: String,
    pub provider: String,
    pub surface: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub kind: ChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_date: Option<String>,
    pub confidence: Confidence,
    pub source: ChangeSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_url: Option<String>,
}

/// One place in the scanned codebase that touches a provider surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSite {
    pub schema_version: String,
    pub provider: String,
    pub surface: String,
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Opportunity,
    Deprecated,
    BreaksLater,
    BreaksNow,
}

/// A ChangeEvent joined to a UsageSite it affects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Impact {
    pub schema_version: String,
    pub severity: Severity,
    pub change: ChangeEvent,
    pub site: UsageSite,
}

/// How scan recognizes a provider in source code: constructor names
/// that bind client variables, and module-level root identifiers.
/// Ships as registry data so new providers need no core changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSpec {
    pub schema_version: String,
    pub name: String,
    pub constructors: Vec<String>,
    pub roots: Vec<String>,
}

/// A provider-authored fix, shipped as data alongside a changefeed.
/// Matches an impact by provider and surface, optionally narrowed by
/// field and change kind. Steps run in order against the site line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub schema_version: String,
    pub provider: String,
    pub surface: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<ChangeKind>,
    pub steps: Vec<RecipeStep>,
}

/// Optional steps apply where they can and skip where they cannot,
/// for fields not every call site passes. Required steps fail the
/// whole recipe when they cannot apply cleanly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RecipeStep {
    RenameSurface {
        to: String,
    },
    RenameField {
        from: String,
        to: String,
        #[serde(default)]
        optional: bool,
    },
    SetField {
        name: String,
        value: String,
    },
    RemoveField {
        name: String,
        #[serde(default)]
        optional: bool,
    },
}

impl RecipeStep {
    pub fn is_optional(&self) -> bool {
        match self {
            RecipeStep::RenameField { optional, .. } | RecipeStep::RemoveField { optional, .. } => {
                *optional
            }
            _ => false,
        }
    }
}

/// A proposed line edit. Proposed means unproven: nothing is applied
/// unless the precondition held at mend time, and nothing should merge
/// without the prove gate downstream. `origin` records which tier
/// produced it so PRs can label synthesized patches honestly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchCandidate {
    pub schema_version: String,
    pub file: String,
    pub line: usize,
    pub old_line: String,
    pub new_line: String,
    #[serde(default = "default_origin")]
    pub origin: String,
    pub impact: Impact,
}

fn default_origin() -> String {
    "deterministic".into()
}

/// An impact mend looked at and declined to patch, with the reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Refusal {
    pub schema_version: String,
    pub reason: String,
    pub impact: Impact,
}

pub fn read_ndjson<T: for<'de> Deserialize<'de>>(reader: impl BufRead) -> Result<Vec<T>> {
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str(&line)?);
    }
    Ok(out)
}

pub fn write_ndjson<T: Serialize>(mut writer: impl Write, items: &[T]) -> Result<()> {
    for item in items {
        serde_json::to_writer(&mut writer, item)?;
        writeln!(writer)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_event_roundtrip() {
        let event = ChangeEvent {
            schema_version: SCHEMA_VERSION.into(),
            provider: "stripe".into(),
            surface: "charges.create".into(),
            field: Some("amount".into()),
            kind: ChangeKind::TypeChanged,
            from: Some("integer".into()),
            to: Some("string".into()),
            effective_version: Some("2026-06-30".into()),
            effective_date: None,
            confidence: Confidence::High,
            source: ChangeSource::OpenapiDiff,
            evidence_url: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: ChangeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.surface, "charges.create");
        assert_eq!(back.kind, ChangeKind::TypeChanged);
    }

    #[test]
    fn ndjson_roundtrip() {
        let sites = vec![UsageSite {
            schema_version: SCHEMA_VERSION.into(),
            provider: "stripe".into(),
            surface: "charges.create".into(),
            file: "src/billing.ts".into(),
            line: 42,
            column: 7,
            language: "typescript".into(),
            snippet: None,
        }];
        let mut buf = Vec::new();
        write_ndjson(&mut buf, &sites).unwrap();
        let back: Vec<UsageSite> = read_ndjson(buf.as_slice()).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].line, 42);
    }
}
