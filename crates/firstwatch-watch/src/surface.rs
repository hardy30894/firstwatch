/// Maps a REST operation to the SDK-style surface the scanner emits.
/// `POST /v1/charges` becomes `charges.create`, `GET /v1/charges/{id}` becomes `charges.retrieve`.
/// Nested resources join with underscores the way stripe-node does:
/// `GET /v1/checkout/sessions` becomes `checkout.sessions.list`.
pub fn sdk_surface(method: &str, path: &str) -> String {
    let segments: Vec<&str> = path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty() && *s != "v1")
        .collect();
    let on_item = segments.last().is_some_and(|s| s.starts_with('{'));
    let resource: Vec<&str> = segments
        .iter()
        .filter(|s| !s.starts_with('{'))
        .copied()
        .collect();
    let verb = match (method.to_uppercase().as_str(), on_item) {
        ("GET", false) => "list",
        ("GET", true) => "retrieve",
        ("POST", false) => "create",
        ("POST", true) => "update",
        ("DELETE", true) => "del",
        ("DELETE", false) => "del",
        ("PUT", _) | ("PATCH", _) => "update",
        _ => "call",
    };
    format!("{}.{}", resource.join("."), verb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_and_item_verbs() {
        assert_eq!(sdk_surface("POST", "/v1/charges"), "charges.create");
        assert_eq!(sdk_surface("GET", "/v1/charges"), "charges.list");
        assert_eq!(sdk_surface("GET", "/v1/charges/{charge}"), "charges.retrieve");
        assert_eq!(sdk_surface("POST", "/v1/charges/{charge}"), "charges.update");
        assert_eq!(sdk_surface("DELETE", "/v1/customers/{id}"), "customers.del");
    }

    #[test]
    fn nested_resources() {
        assert_eq!(
            sdk_surface("GET", "/v1/checkout/sessions"),
            "checkout.sessions.list"
        );
        assert_eq!(
            sdk_surface("POST", "/v1/customers/{id}/sources"),
            "customers.sources.create"
        );
    }
}
