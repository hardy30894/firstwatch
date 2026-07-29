mod py;
mod ts;

use std::path::Path;

use anyhow::{Context, Result};
use firstwatch_schema::UsageSite;
use walkdir::WalkDir;

pub fn scan_dir(root: &Path) -> Result<Vec<UsageSite>> {
    let mut sites = Vec::new();
    for entry in WalkDir::new(root).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        name != "node_modules" && name != ".git" && name != ".venv" && name != "__pycache__"
    }) {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type().is_file() || !is_supported(path) {
            continue;
        }
        let source =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let rel = path.strip_prefix(root).unwrap_or(path);
        sites.extend(scan_source(&source, &rel.to_string_lossy())?);
    }
    Ok(sites)
}

pub fn scan_source(source: &str, file: &str) -> Result<Vec<UsageSite>> {
    match extension(file) {
        Some("py") => py::scan(source, file),
        Some("ts") | Some("tsx") | Some("js") | Some("mjs") => ts::scan(source, file),
        _ => Ok(Vec::new()),
    }
}

fn is_supported(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("py") | Some("ts") | Some("tsx") | Some("js") | Some("mjs")
    )
}

fn extension(file: &str) -> Option<&str> {
    Path::new(file).extension().and_then(|e| e.to_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_stripe_calls_from_new_binding() {
        let src = r#"
import Stripe from "stripe";
const client = new Stripe(key);
const charge = await client.charges.create({ amount: 100 });
"#;
        let sites = scan_source(src, "billing.ts").unwrap();
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].surface, "charges.create");
        assert_eq!(sites[0].line, 4);
    }

    #[test]
    fn finds_calls_on_var_named_stripe_without_binding() {
        let src = "await stripe.customers.list({ limit: 10 });";
        let sites = scan_source(src, "a.ts").unwrap();
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].surface, "customers.list");
    }

    #[test]
    fn ignores_unrelated_receivers() {
        let src = r#"
const db = new Pool(cfg);
await db.users.findMany({});
"#;
        let sites = scan_source(src, "a.ts").unwrap();
        assert!(sites.is_empty());
    }

    #[test]
    fn dispatches_python_by_extension() {
        let src = "client = StripeClient(key)\nclient.charges.retrieve(id)\n";
        let sites = scan_source(src, "app.py").unwrap();
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].language, "python");
    }

    #[test]
    fn unknown_extension_is_empty() {
        let sites = scan_source("stripe.charges.create()", "notes.txt").unwrap();
        assert!(sites.is_empty());
    }
}
