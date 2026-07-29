use firstwatch_schema::{ProviderSpec, SCHEMA_VERSION};

/// The active provider set: built-ins by default, extended or replaced
/// by ProviderSpec data from a registry file.
pub struct ProviderSet {
    specs: Vec<ProviderSpec>,
}

impl Default for ProviderSet {
    fn default() -> Self {
        ProviderSet { specs: builtins() }
    }
}

impl ProviderSet {
    pub fn with_extra(extra: Vec<ProviderSpec>) -> Self {
        let mut specs = builtins();
        for spec in extra {
            specs.retain(|s| s.name != spec.name);
            specs.push(spec);
        }
        ProviderSet { specs }
    }

    pub fn by_constructor(&self, ctor: &str) -> Option<&ProviderSpec> {
        self.specs
            .iter()
            .find(|p| p.constructors.iter().any(|c| c == ctor))
    }

    pub fn by_root(&self, root: &str) -> Option<&ProviderSpec> {
        let lower = root.to_lowercase();
        self.specs.iter().find(|p| p.roots.contains(&lower))
    }
}

fn builtins() -> Vec<ProviderSpec> {
    let spec = |name: &str, constructors: &[&str], roots: &[&str]| ProviderSpec {
        schema_version: SCHEMA_VERSION.into(),
        name: name.into(),
        constructors: constructors.iter().map(|s| s.to_string()).collect(),
        roots: roots.iter().map(|s| s.to_string()).collect(),
    };
    vec![
        spec("stripe", &["Stripe", "StripeClient"], &["stripe"]),
        spec("openai", &["OpenAI", "AsyncOpenAI"], &["openai"]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_specs_extend_and_override_builtins() {
        let set = ProviderSet::with_extra(vec![ProviderSpec {
            schema_version: SCHEMA_VERSION.into(),
            name: "github".into(),
            constructors: vec!["Octokit".into()],
            roots: vec!["octokit".into()],
        }]);
        assert_eq!(set.by_constructor("Octokit").unwrap().name, "github");
        assert_eq!(set.by_constructor("Stripe").unwrap().name, "stripe");
        assert_eq!(set.by_root("openai").unwrap().name, "openai");
    }
}
