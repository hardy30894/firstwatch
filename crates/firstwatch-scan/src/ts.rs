use std::collections::HashMap;

use anyhow::{Context, Result};
use firstwatch_schema::{UsageSite, SCHEMA_VERSION};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::providers::ProviderSet;

const CALL_QUERY: &str = "(call_expression function: (member_expression) @fn)";

const BINDING_QUERY: &str = r#"
(variable_declarator
  name: (identifier) @name
  value: (new_expression constructor: (identifier) @ctor))
"#;

pub fn scan(source: &str, file: &str, providers: &ProviderSet) -> Result<Vec<UsageSite>> {
    let language = tree_sitter::Language::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT);
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
            let Some((root, chain, node)) = member_chain(cap.node, src) else {
                continue;
            };
            if chain.len() < 2 {
                continue;
            }
            let provider = bound_vars
                .get(&root)
                .cloned()
                .or_else(|| providers.by_root(&root).map(|p| p.name.clone()));
            let Some(provider) = provider else { continue };
            let pos = node.start_position();
            sites.push(UsageSite {
                schema_version: SCHEMA_VERSION.into(),
                provider: provider.clone(),
                surface: chain.join("."),
                file: file.to_string(),
                line: pos.row + 1,
                column: pos.column + 1,
                language: "typescript".into(),
                snippet: source.lines().nth(pos.row).map(|l| l.trim().to_string()),
            });
        }
    }
    Ok(sites)
}

/// Decomposes a member expression into its root identifier and the
/// property chain: client.chat.completions.create gives
/// ("client", ["chat", "completions", "create"]).
fn member_chain<'a>(node: Node<'a>, src: &[u8]) -> Option<(String, Vec<String>, Node<'a>)> {
    let mut chain = Vec::new();
    let mut current = node;
    loop {
        let prop = current.child_by_field_name("property")?;
        chain.push(prop.utf8_text(src).ok()?.to_string());
        let object = current.child_by_field_name("object")?;
        match object.kind() {
            "member_expression" => current = object,
            "identifier" => {
                chain.reverse();
                let root = object.utf8_text(src).ok()?.to_string();
                return Some((root, chain, object));
            }
            _ => return None,
        }
    }
}
