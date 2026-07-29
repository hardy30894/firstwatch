use std::collections::HashSet;

use anyhow::{Context, Result};
use firstwatch_schema::{UsageSite, SCHEMA_VERSION};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

const CALL_QUERY: &str = r#"
(call_expression
  function: (member_expression
    object: (member_expression
      object: (identifier) @root
      property: (property_identifier) @group)
    property: (property_identifier) @method))
"#;

const BINDING_QUERY: &str = r#"
(variable_declarator
  name: (identifier) @name
  value: (new_expression constructor: (identifier) @ctor))
"#;

pub fn scan(source: &str, file: &str) -> Result<Vec<UsageSite>> {
    let language = tree_sitter::Language::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT);
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let tree = parser
        .parse(source, None)
        .context("tree-sitter failed to parse")?;
    let src = source.as_bytes();

    let binding_query = Query::new(&language, BINDING_QUERY)?;
    let mut stripe_vars: HashSet<String> = HashSet::new();
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
        if ctor.as_deref() == Some("Stripe") {
            if let Some(name) = name {
                stripe_vars.insert(name);
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
        if !stripe_vars.contains(&root) && root.to_lowercase() != "stripe" {
            continue;
        }
        sites.push(UsageSite {
            schema_version: SCHEMA_VERSION.into(),
            provider: "stripe".into(),
            surface: format!("{group}.{method}"),
            file: file.to_string(),
            line: pos.row + 1,
            column: pos.column + 1,
            language: "typescript".into(),
            snippet: source.lines().nth(pos.row).map(|l| l.trim().to_string()),
        });
    }
    Ok(sites)
}
