use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use firstwatch_schema::{read_ndjson, Impact, PatchCandidate, ProvenFix, SCHEMA_VERSION};

/// Local store of proven fixes at ~/.firstwatch/proven.ndjson
/// (FIRSTWATCH_HOME overrides the directory). Contributable to the
/// registry as-is via `firstwatch share --proven`.
pub fn store_path() -> Option<PathBuf> {
    let dir = std::env::var("FIRSTWATCH_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".firstwatch"))
                .ok()
        })?;
    Some(dir.join("proven.ndjson"))
}

pub fn template_key_parts(
    provider: &str,
    surface: &str,
    kind: &firstwatch_schema::ChangeKind,
    field: Option<&str>,
) -> String {
    format!("{provider}|{surface}|{kind:?}|{}", field.unwrap_or(""))
}

pub fn key_of_impact(impact: &Impact) -> String {
    template_key_parts(
        &impact.change.provider,
        &impact.change.surface,
        &impact.change.kind,
        impact.change.field.as_deref(),
    )
}

/// Templates for propagation: change identity -> (old line, new line), trimmed.
pub fn load_templates() -> HashMap<String, (String, String)> {
    let mut templates = HashMap::new();
    let Some(path) = store_path() else {
        return templates;
    };
    let Ok(file) = std::fs::File::open(&path) else {
        return templates;
    };
    let fixes: Vec<ProvenFix> = read_ndjson(std::io::BufReader::new(file)).unwrap_or_default();
    for fix in fixes {
        templates.insert(
            template_key_parts(&fix.provider, &fix.surface, &fix.kind, fix.field.as_deref()),
            (fix.old_line, fix.new_line),
        );
    }
    templates
}

/// Records proven patches, deduplicated by change identity and old line.
pub fn record_proven(patches: &[PatchCandidate], checks: &[String]) -> Result<usize> {
    let Some(path) = store_path() else {
        return Ok(0);
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut existing: HashMap<String, ProvenFix> = HashMap::new();
    if let Ok(file) = std::fs::File::open(&path) {
        let fixes: Vec<ProvenFix> = read_ndjson(std::io::BufReader::new(file)).unwrap_or_default();
        for fix in fixes {
            let key = format!(
                "{}|{}",
                template_key_parts(&fix.provider, &fix.surface, &fix.kind, fix.field.as_deref()),
                fix.old_line
            );
            existing.insert(key, fix);
        }
    }
    let mut added = 0;
    for patch in patches {
        let change = &patch.impact.change;
        let fix = ProvenFix {
            schema_version: SCHEMA_VERSION.into(),
            provider: change.provider.clone(),
            surface: change.surface.clone(),
            kind: change.kind.clone(),
            field: change.field.clone(),
            old_line: patch.old_line.trim().to_string(),
            new_line: patch.new_line.trim().to_string(),
            checks: checks.to_vec(),
        };
        let key = format!("{}|{}", key_of_impact(&patch.impact), fix.old_line);
        if existing.insert(key, fix).is_none() {
            added += 1;
        }
    }
    let mut fixes: Vec<&ProvenFix> = existing.values().collect();
    fixes.sort_by(|a, b| {
        (&a.provider, &a.surface, &a.old_line).cmp(&(&b.provider, &b.surface, &b.old_line))
    });
    let mut out = Vec::new();
    firstwatch_schema::write_ndjson(&mut out, &fixes)?;
    std::fs::write(&path, out)?;
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use firstwatch_schema::{
        ChangeEvent, ChangeKind, ChangeSource, Confidence, Severity, UsageSite,
    };

    fn patch() -> PatchCandidate {
        PatchCandidate {
            schema_version: SCHEMA_VERSION.into(),
            file: "a.ts".into(),
            line: 1,
            old_line: "  old();".into(),
            new_line: "  new();".into(),
            origin: "tier1".into(),
            impact: Impact {
                schema_version: SCHEMA_VERSION.into(),
                severity: Severity::BreaksNow,
                change: ChangeEvent {
                    schema_version: SCHEMA_VERSION.into(),
                    provider: "stripe".into(),
                    surface: "charges.create".into(),
                    field: None,
                    kind: ChangeKind::Renamed,
                    from: None,
                    to: Some("payment_intents.create".into()),
                    effective_version: None,
                    effective_date: None,
                    confidence: Confidence::High,
                    source: ChangeSource::OpenapiDiff,
                    evidence_url: None,
                },
                site: UsageSite {
                    schema_version: SCHEMA_VERSION.into(),
                    provider: "stripe".into(),
                    surface: "charges.create".into(),
                    file: "a.ts".into(),
                    line: 1,
                    column: 1,
                    language: "typescript".into(),
                    snippet: None,
                },
            },
        }
    }

    #[test]
    fn records_dedupes_and_loads() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("FIRSTWATCH_HOME", dir.path());
        let added = record_proven(&[patch()], &["typecheck".into()]).unwrap();
        assert_eq!(added, 1);
        let added = record_proven(&[patch()], &["typecheck".into()]).unwrap();
        assert_eq!(added, 0);
        let templates = load_templates();
        assert_eq!(templates.len(), 1);
        let (old, new) = templates.values().next().unwrap();
        assert_eq!(old, "old();");
        assert_eq!(new, "new();");
        std::env::remove_var("FIRSTWATCH_HOME");
    }
}
