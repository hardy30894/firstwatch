use std::collections::BTreeMap;

use anyhow::Result;
use firstwatch_schema::{ChangeEvent, ChangeKind, ChangeSource, Confidence, SCHEMA_VERSION};
use serde_json::Value;

use crate::surface::sdk_surface;

const METHODS: [&str; 5] = ["get", "post", "put", "patch", "delete"];

pub fn diff_openapi(old: &Value, new: &Value, provider: &str) -> Result<Vec<ChangeEvent>> {
    let old_ops = collect_operations(old);
    let new_ops = collect_operations(new);
    let version = new
        .pointer("/info/version")
        .and_then(Value::as_str)
        .map(String::from);
    let mut events = Vec::new();

    for (key, old_op) in &old_ops {
        let surface = sdk_surface(&key.0, &key.1);
        match new_ops.get(key) {
            None => events.push(event(
                provider,
                &surface,
                None,
                ChangeKind::Removed,
                None,
                None,
                &version,
            )),
            Some(new_op) => {
                diff_operation(provider, &surface, old_op, new_op, &version, &mut events);
            }
        }
    }
    for key in new_ops.keys() {
        if !old_ops.contains_key(key) {
            let surface = sdk_surface(&key.0, &key.1);
            events.push(event(
                provider,
                &surface,
                None,
                ChangeKind::Added,
                None,
                None,
                &version,
            ));
        }
    }
    Ok(events)
}

fn diff_operation(
    provider: &str,
    surface: &str,
    old_op: &Value,
    new_op: &Value,
    version: &Option<String>,
    events: &mut Vec<ChangeEvent>,
) {
    if new_op.get("deprecated").and_then(Value::as_bool) == Some(true)
        && old_op.get("deprecated").and_then(Value::as_bool) != Some(true)
    {
        events.push(event(
            provider,
            surface,
            None,
            ChangeKind::Deprecated,
            None,
            None,
            version,
        ));
    }

    let old_props = request_properties(old_op);
    let new_props = request_properties(new_op);
    let old_required = request_required(old_op);
    let new_required = request_required(new_op);

    for (name, old_schema) in &old_props {
        match new_props.get(name) {
            None => events.push(event(
                provider,
                surface,
                Some(name),
                ChangeKind::Removed,
                None,
                None,
                version,
            )),
            Some(new_schema) => {
                let old_type = schema_type(old_schema);
                let new_type = schema_type(new_schema);
                if old_type != new_type {
                    events.push(event(
                        provider,
                        surface,
                        Some(name),
                        ChangeKind::TypeChanged,
                        old_type,
                        new_type,
                        version,
                    ));
                }
            }
        }
    }
    for name in new_required {
        if !old_required.contains(&name) && new_props.contains_key(&name) {
            events.push(event(
                provider,
                surface,
                Some(&name),
                ChangeKind::RequiredAdded,
                None,
                None,
                version,
            ));
        }
    }
}

type OpKey = (String, String);

fn collect_operations(spec: &Value) -> BTreeMap<OpKey, Value> {
    let mut ops = BTreeMap::new();
    let Some(paths) = spec.get("paths").and_then(Value::as_object) else {
        return ops;
    };
    for (path, item) in paths {
        let Some(item) = item.as_object() else { continue };
        for method in METHODS {
            if let Some(op) = item.get(method) {
                ops.insert((method.to_string(), path.clone()), op.clone());
            }
        }
    }
    ops
}

fn request_properties(op: &Value) -> BTreeMap<String, Value> {
    let mut props = BTreeMap::new();
    let Some(content) = op.pointer("/requestBody/content").and_then(Value::as_object) else {
        return props;
    };
    for body in content.values() {
        if let Some(map) = body.pointer("/schema/properties").and_then(Value::as_object) {
            for (k, v) in map {
                props.insert(k.clone(), v.clone());
            }
        }
    }
    props
}

fn request_required(op: &Value) -> Vec<String> {
    let Some(content) = op.pointer("/requestBody/content").and_then(Value::as_object) else {
        return Vec::new();
    };
    content
        .values()
        .filter_map(|body| body.pointer("/schema/required").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect()
}

fn schema_type(schema: &Value) -> Option<String> {
    schema.get("type").and_then(Value::as_str).map(String::from)
}

fn event(
    provider: &str,
    surface: &str,
    field: Option<&str>,
    kind: ChangeKind,
    from: Option<String>,
    to: Option<String>,
    version: &Option<String>,
) -> ChangeEvent {
    ChangeEvent {
        schema_version: SCHEMA_VERSION.into(),
        provider: provider.into(),
        surface: surface.into(),
        field: field.map(String::from),
        kind,
        from,
        to,
        effective_version: version.clone(),
        effective_date: None,
        confidence: Confidence::High,
        source: ChangeSource::OpenapiDiff,
        evidence_url: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(paths: Value) -> Value {
        json!({ "info": { "version": "2026-07-01" }, "paths": paths })
    }

    #[test]
    fn removed_operation() {
        let old = spec(json!({ "/v1/orders": { "post": {} } }));
        let new = spec(json!({}));
        let events = diff_openapi(&old, &new, "stripe").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].surface, "orders.create");
        assert_eq!(events[0].kind, ChangeKind::Removed);
        assert_eq!(events[0].effective_version.as_deref(), Some("2026-07-01"));
    }

    #[test]
    fn removed_field_and_type_change() {
        let body = |props: Value| {
            json!({ "requestBody": { "content": { "application/x-www-form-urlencoded": {
                "schema": { "properties": props }
            }}}})
        };
        let old = spec(json!({ "/v1/charges": { "post":
            body(json!({ "amount": { "type": "integer" }, "legacy_flag": { "type": "boolean" } })) } }));
        let new = spec(json!({ "/v1/charges": { "post":
            body(json!({ "amount": { "type": "string" } })) } }));
        let events = diff_openapi(&old, &new, "stripe").unwrap();
        assert_eq!(events.len(), 2);
        assert!(events.iter().any(|e| e.kind == ChangeKind::Removed
            && e.field.as_deref() == Some("legacy_flag")));
        assert!(events.iter().any(|e| e.kind == ChangeKind::TypeChanged
            && e.field.as_deref() == Some("amount")
            && e.from.as_deref() == Some("integer")
            && e.to.as_deref() == Some("string")));
    }

    #[test]
    fn newly_required_field() {
        let old = spec(json!({ "/v1/payouts": { "post": { "requestBody": { "content": {
            "application/json": { "schema": {
                "properties": { "currency": { "type": "string" } }, "required": [] } } } } } } }));
        let new = spec(json!({ "/v1/payouts": { "post": { "requestBody": { "content": {
            "application/json": { "schema": {
                "properties": { "currency": { "type": "string" } }, "required": ["currency"] } } } } } } }));
        let events = diff_openapi(&old, &new, "stripe").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ChangeKind::RequiredAdded);
        assert_eq!(events[0].field.as_deref(), Some("currency"));
    }

    #[test]
    fn deprecation_flag() {
        let old = spec(json!({ "/v1/charges": { "post": {} } }));
        let new = spec(json!({ "/v1/charges": { "post": { "deprecated": true } } }));
        let events = diff_openapi(&old, &new, "stripe").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ChangeKind::Deprecated);
    }

    #[test]
    fn added_operation() {
        let old = spec(json!({}));
        let new = spec(json!({ "/v1/meters": { "get": {} } }));
        let events = diff_openapi(&old, &new, "stripe").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].surface, "meters.list");
        assert_eq!(events[0].kind, ChangeKind::Added);
    }
}
