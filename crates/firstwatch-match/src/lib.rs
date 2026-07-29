use firstwatch_schema::{ChangeEvent, ChangeKind, Impact, Severity, UsageSite, SCHEMA_VERSION};

pub fn match_impacts(events: &[ChangeEvent], sites: &[UsageSite]) -> Vec<Impact> {
    let mut impacts = Vec::new();
    for event in events {
        for site in sites {
            if site.provider == event.provider && site.surface == event.surface {
                impacts.push(Impact {
                    schema_version: SCHEMA_VERSION.into(),
                    severity: severity_of(event),
                    change: event.clone(),
                    site: site.clone(),
                });
            }
        }
    }
    impacts.sort_by(|a, b| b.severity.cmp(&a.severity));
    impacts
}

fn severity_of(event: &ChangeEvent) -> Severity {
    match event.kind {
        ChangeKind::Removed | ChangeKind::Renamed | ChangeKind::TypeChanged => {
            if event.effective_date.is_some() {
                Severity::BreaksLater
            } else {
                Severity::BreaksNow
            }
        }
        ChangeKind::RequiredAdded => Severity::BreaksLater,
        ChangeKind::Deprecated => Severity::Deprecated,
        ChangeKind::Added => Severity::Opportunity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firstwatch_schema::{ChangeSource, Confidence};

    fn event(surface: &str, kind: ChangeKind) -> ChangeEvent {
        ChangeEvent {
            schema_version: SCHEMA_VERSION.into(),
            provider: "stripe".into(),
            surface: surface.into(),
            field: None,
            kind,
            from: None,
            to: None,
            effective_version: None,
            effective_date: None,
            confidence: Confidence::High,
            source: ChangeSource::OpenapiDiff,
            evidence_url: None,
        }
    }

    fn site(surface: &str) -> UsageSite {
        UsageSite {
            schema_version: SCHEMA_VERSION.into(),
            provider: "stripe".into(),
            surface: surface.into(),
            file: "src/billing.ts".into(),
            line: 1,
            column: 1,
            language: "typescript".into(),
            snippet: None,
        }
    }

    #[test]
    fn joins_on_provider_and_surface() {
        let impacts = match_impacts(
            &[event("charges.create", ChangeKind::Removed)],
            &[site("charges.create"), site("customers.list")],
        );
        assert_eq!(impacts.len(), 1);
        assert_eq!(impacts[0].severity, Severity::BreaksNow);
    }

    #[test]
    fn no_match_no_impact() {
        let impacts = match_impacts(
            &[event("charges.create", ChangeKind::Removed)],
            &[site("customers.list")],
        );
        assert!(impacts.is_empty());
    }

    #[test]
    fn breaking_kinds_outrank_deprecations() {
        let impacts = match_impacts(
            &[
                event("customers.list", ChangeKind::Deprecated),
                event("charges.create", ChangeKind::Removed),
            ],
            &[site("charges.create"), site("customers.list")],
        );
        assert_eq!(impacts[0].severity, Severity::BreaksNow);
        assert_eq!(impacts[1].severity, Severity::Deprecated);
    }
}
