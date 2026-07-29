use std::collections::HashSet;

use anyhow::{Context, Result};
use firstwatch_schema::{UsageSite, SCHEMA_VERSION};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

const CALL_QUERY: &str = r#"
(call
  function: (attribute
    object: (attribute
      object: (identifier) @root
      attribute: (identifier) @group)
    attribute: (identifier) @method))
"#;

const BINDING_QUERY: &str = r#"
(assignment
  left: (identifier) @name
  right: (call function: (identifier) @ctor))
"#;

pub fn scan(source: &str, file: &str) -> Result<Vec<UsageSite>> {
    let language = tree_sitter::Language::new(tree_sitter_python::LANGUAGE);
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let tree = parser
        .parse(source, None)
        .context("tree-sitter failed to parse")?;
    let src = source.as_bytes();

    let binding_query = Query::new(&language, BINDING_QUERY)?;
    let mut client_vars: HashSet<String> = HashSet::new();
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
        if ctor.as_deref() == Some("StripeClient") {
            if let Some(name) = name {
                client_vars.insert(name);
            }
        }
    }

    let call_query = Query::new(&language, CALL_QUERY)?;
    let mut sites = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&call_query, tree.root_node(), src);
    while let Some(m) = matches.next() {
        let mut root = None;
        let mut group = None;
        let mut method = None;
        let mut position = None;
        for cap in m.captures {
            let text = cap.node.utf8_text(src)?;
            match call_query.capture_names()[cap.index as usize] {
                "root" => {
                    root = Some(text.to_string());
                    position = Some(cap.node.start_position());
                }
                "group" => group = Some(text.to_string()),
                "method" => method = Some(text.to_string()),
                _ => {}
            }
        }
        let (Some(root), Some(group), Some(method), Some(pos)) = (root, group, method, position)
        else {
            continue;
        };
        let surface = if client_vars.contains(&root) {
            format!("{group}.{method}")
        } else if root == "stripe" && group.starts_with(char::is_uppercase) {
            format!("{}.{method}", legacy_resource(&group))
        } else if root == "stripe" || root == "client" {
            format!("{group}.{method}")
        } else {
            continue;
        };
        sites.push(UsageSite {
            schema_version: SCHEMA_VERSION.into(),
            provider: "stripe".into(),
            surface,
            file: file.to_string(),
            line: pos.row + 1,
            column: pos.column + 1,
            language: "python".into(),
            snippet: source.lines().nth(pos.row).map(|l| l.trim().to_string()),
        });
    }
    Ok(sites)
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
        let sites = scan(src, "billing.py").unwrap();
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].surface, "payment_intents.create");
        assert_eq!(sites[0].language, "python");
        assert_eq!(sites[0].line, 2);
    }

    #[test]
    fn maps_legacy_class_calls() {
        let src = "import stripe\ncharge = stripe.Charge.create(amount=100)\nintent = stripe.PaymentIntent.create(amount=100)\n";
        let sites = scan(src, "billing.py").unwrap();
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].surface, "charges.create");
        assert_eq!(sites[1].surface, "payment_intents.create");
    }

    #[test]
    fn ignores_unrelated_receivers() {
        let src = "db = Pool(cfg)\nrows = db.users.fetch(limit=1)\n";
        let sites = scan(src, "app.py").unwrap();
        assert!(sites.is_empty());
    }
}
