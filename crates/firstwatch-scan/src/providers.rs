/// The provider table. Adding a provider here is the only core change
/// scan needs; everything else ships as changefeed and recipe data.
pub struct Provider {
    pub name: &'static str,
    pub constructors: &'static [&'static str],
    pub roots: &'static [&'static str],
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        name: "stripe",
        constructors: &["Stripe", "StripeClient"],
        roots: &["stripe"],
    },
    Provider {
        name: "openai",
        constructors: &["OpenAI", "AsyncOpenAI"],
        roots: &["openai"],
    },
];

pub fn by_constructor(ctor: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.constructors.contains(&ctor))
}

pub fn by_root(root: &str) -> Option<&'static Provider> {
    let lower = root.to_lowercase();
    PROVIDERS.iter().find(|p| p.roots.contains(&lower.as_str()))
}
