use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use firstwatch_schema::{ChangeKind, Impact, PatchCandidate, Refusal, SCHEMA_VERSION};

pub struct MendOutcome {
    pub patches: Vec<PatchCandidate>,
    pub refusals: Vec<Refusal>,
}

pub fn mend(root: &Path, impacts: &[Impact]) -> Result<MendOutcome> {
    let mut files: HashMap<String, Vec<String>> = HashMap::new();
    let mut patches = Vec::new();
    let mut refusals = Vec::new();

    for impact in impacts {
        let Some(replacement) = replacement_surface(impact) else {
            refusals.push(refuse(
                impact,
                "no deterministic rewrite for this change kind",
            ));
            continue;
        };
        let lines = match files.entry(impact.site.file.clone()) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let content = std::fs::read_to_string(root.join(&impact.site.file))?;
                e.insert(content.lines().map(String::from).collect())
            }
        };
        let Some(old_line) = lines.get(impact.site.line - 1) else {
            refusals.push(refuse(impact, "site line no longer exists"));
            continue;
        };
        if !old_line.contains(&impact.site.surface) {
            refusals.push(refuse(impact, "site line no longer contains the surface"));
            continue;
        }
        let new_line = old_line.replace(&impact.site.surface, &replacement);
        patches.push(PatchCandidate {
            schema_version: SCHEMA_VERSION.into(),
            file: impact.site.file.clone(),
            line: impact.site.line,
            old_line: old_line.clone(),
            new_line,
            impact: impact.clone(),
        });
    }
    Ok(MendOutcome { patches, refusals })
}

pub fn apply(root: &Path, patches: &[PatchCandidate]) -> Result<usize> {
    let mut by_file: HashMap<&str, Vec<&PatchCandidate>> = HashMap::new();
    for patch in patches {
        by_file.entry(&patch.file).or_default().push(patch);
    }
    let mut applied = 0;
    for (file, patches) in by_file {
        let path = root.join(file);
        let content = std::fs::read_to_string(&path)?;
        let mut lines: Vec<String> = content.lines().map(String::from).collect();
        for patch in patches {
            let idx = patch.line - 1;
            if lines.get(idx).map(String::as_str) == Some(patch.old_line.as_str()) {
                lines[idx] = patch.new_line.clone();
                applied += 1;
            }
        }
        let trailing = if content.ends_with('\n') { "\n" } else { "" };
        std::fs::write(&path, lines.join("\n") + trailing)?;
    }
    Ok(applied)
}

fn replacement_surface(impact: &Impact) -> Option<String> {
    match impact.change.kind {
        ChangeKind::Renamed | ChangeKind::Deprecated => impact.change.to.clone(),
        _ => None,
    }
}

fn refuse(impact: &Impact, reason: &str) -> Refusal {
    Refusal {
        schema_version: SCHEMA_VERSION.into(),
        reason: reason.into(),
        impact: impact.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firstwatch_schema::{ChangeEvent, ChangeSource, Confidence, Severity, UsageSite};

    fn impact(kind: ChangeKind, to: Option<&str>, file: &str, line: usize) -> Impact {
        Impact {
            schema_version: SCHEMA_VERSION.into(),
            severity: Severity::Deprecated,
            change: ChangeEvent {
                schema_version: SCHEMA_VERSION.into(),
                provider: "stripe".into(),
                surface: "charges.create".into(),
                field: None,
                kind,
                from: None,
                to: to.map(String::from),
                effective_version: None,
                effective_date: None,
                confidence: Confidence::High,
                source: ChangeSource::Changelog,
                evidence_url: None,
            },
            site: UsageSite {
                schema_version: SCHEMA_VERSION.into(),
                provider: "stripe".into(),
                surface: "charges.create".into(),
                file: file.into(),
                line,
                column: 1,
                language: "typescript".into(),
                snippet: None,
            },
        }
    }

    fn setup(dir: &Path, content: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("billing.ts"), content).unwrap();
    }

    #[test]
    fn rewrites_deprecated_surface_with_replacement() {
        let dir = std::env::temp_dir().join("fw-mend-rewrite");
        setup(
            &dir,
            "const c = await stripe.charges.create({ amount: 1 });\n",
        );
        let outcome = mend(
            &dir,
            &[impact(
                ChangeKind::Deprecated,
                Some("payment_intents.create"),
                "billing.ts",
                1,
            )],
        )
        .unwrap();
        assert_eq!(outcome.patches.len(), 1);
        assert!(outcome.patches[0]
            .new_line
            .contains("stripe.payment_intents.create"));

        let applied = apply(&dir, &outcome.patches).unwrap();
        assert_eq!(applied, 1);
        let content = std::fs::read_to_string(dir.join("billing.ts")).unwrap();
        assert!(content.contains("payment_intents.create"));
        assert!(!content.contains("charges.create"));
    }

    #[test]
    fn refuses_removal_without_replacement() {
        let dir = std::env::temp_dir().join("fw-mend-refuse-kind");
        setup(&dir, "await stripe.charges.create({});\n");
        let outcome = mend(&dir, &[impact(ChangeKind::Removed, None, "billing.ts", 1)]).unwrap();
        assert!(outcome.patches.is_empty());
        assert_eq!(outcome.refusals.len(), 1);
    }

    #[test]
    fn refuses_when_line_drifted() {
        let dir = std::env::temp_dir().join("fw-mend-refuse-drift");
        setup(&dir, "const x = 1;\n");
        let outcome = mend(
            &dir,
            &[impact(
                ChangeKind::Deprecated,
                Some("payment_intents.create"),
                "billing.ts",
                1,
            )],
        )
        .unwrap();
        assert!(outcome.patches.is_empty());
        assert_eq!(
            outcome.refusals[0].reason,
            "site line no longer contains the surface"
        );
    }
}
