use std::collections::HashMap;

use anyhow::{Context, Result};
use firstwatch_schema::{UsageSite, SCHEMA_VERSION};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::providers::ProviderSet;

const CALL_QUERY: &str = "(call function: (attribute) @fn)";

const BINDING_QUERY: &str = r#"
(assignment
  left: (identifier) @name
  right: (call function: (identifier) @ctor))
"#;

pub fn scan(source: &str, file: &str, providers: &ProviderSet) -> Result<Vec<UsageSite>> {
    let language = tree_sitter::Language::new(tree_sitter_python::LANGUAGE);
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let tree = parser
        .parse(source, None)
        .context("tree-sitter failed to parse")?;
    let src = source.as_bytes();

    let binding_query = Query::new(&language, BINDING_QUERY)?;
    let mut bound_vars: HashMap<String, String> = HashMap::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&binding_query, tree.root_node(), src);
    while let Some(m) = matches.next() {
        let mut name = None;
        let mut ctor = None;
        for cap in m.captures {
            let text = cap.node.utf8_text(src)?;
            match binding_query.capture_names()[cap.index as usize] {
                "name" => name = Some(text.to_string()),
                "ctor" => ctor = Some(text.to_string()),
                _ => {}
            }
        }
        if let (Some(name), Some(ctor)) = (name, ctor) {
            if let Some(provider) = providers.by_constructor(&ctor) {
                bound_vars.insert(name, provider.name.clone());
            }
        }
    }

    let call_query = Query::new(&language, CALL_QUERY)?;
    let mut sites = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&call_query, tree.root_node(), src);
    while let Some(m) = matches.next() {
        for cap in m.captures {
            let Some((root, mut chain, node)) = attribute_chain(cap.node, src) else {
                continue;
            };
            if chain.len() < 2 {
                continue;
            }
            let bound = bound_vars.get(&root).cloned();
            let is_bound = bound.is_some();
            let provider = bound.or_else(|| providers.by_root(&root).map(|p| p.name.clone()));
            let Some(provider) = provider else { continue };
            if !is_bound && chain[0].starts_with(char::is_uppercase) {
                chain[0] = legacy_resource(&chain[0]);
            }
            let pos = node.start_position();
            sites.push(UsageSite {
                schema_version: SCHEMA_VERSION.into(),
                provider: provider.clone(),
                surface: chain.join("."),
                file: file.to_string(),
                line: pos.row + 1,
                column: pos.column + 1,
                language: "python".into(),
                snippet: source.lines().nth(pos.row).map(|l| l.trim().to_string()),
            });
        }
    }
    Ok(sites)
}

fn attribute_chain<'a>(node: Node<'a>, src: &[u8]) -> Option<(String, Vec<String>, Node<'a>)> {
    let mut chain = Vec::new();
    let mut current = node;
    loop {
        let attr = current.child_by_field_name("attribute")?;
        chain.push(attr.utf8_text(src).ok()?.to_string());
        let object = current.child_by_field_name("object")?;
        match object.kind() {
            "attribute" => current = object,
            "identifier" => {
                chain.reverse();
                let root = object.utf8_text(src).ok()?.to_string();
                return Some((root, chain, object));
            }
            _ => return None,
        }
    }
}

/// Maps legacy stripe-python class names to modern surfaces:
/// Charge to charges, PaymentIntent to payment_intents.
fn legacy_resource(class_name: &str) -> String {
    let mut out = String::with_capacity(class_name.len() + 4);
    for (i, c) in class_name.char_indices() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out.push('s');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_modern_client_calls() {
        let src =
            "client = StripeClient(key)\nintent = client.payment_intents.create(amount=100)\n";
        let sites = scan(src, "billing.py", &ProviderSet::default()).unwrap();
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].provider, "stripe");
        assert_eq!(sites[0].surface, "payment_intents.create");
        assert_eq!(sites[0].line, 2);
    }

    #[test]
    fn maps_legacy_class_calls() {
        let src = "import stripe\ncharge = stripe.Charge.create(amount=100)\nintent = stripe.PaymentIntent.create(amount=100)\n";
        let sites = scan(src, "billing.py", &ProviderSet::default()).unwrap();
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].surface, "charges.create");
        assert_eq!(sites[1].surface, "payment_intents.create");
    }

    #[test]
    fn finds_openai_client_calls() {
        let src =
            "client = OpenAI()\nresp = client.chat.completions.create(model=m, messages=msgs)\n";
        let sites = scan(src, "app.py", &ProviderSet::default()).unwrap();
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].provider, "openai");
        assert_eq!(sites[0].surface, "chat.completions.create");
    }

    #[test]
    fn finds_openai_module_calls() {
        let src = "import openai\nresp = openai.embeddings.create(input=text)\n";
        let sites = scan(src, "app.py", &ProviderSet::default()).unwrap();
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].provider, "openai");
        assert_eq!(sites[0].surface, "embeddings.create");
    }

    #[test]
    fn ignores_unrelated_receivers() {
        let src = "db = Pool(cfg)\nrows = db.users.fetch(limit=1)\n";
        let sites = scan(src, "app.py", &ProviderSet::default()).unwrap();
        assert!(sites.is_empty());
    }
}
