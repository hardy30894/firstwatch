use std::path::Path;

use firstwatch_scan::ProviderSet;
use firstwatch_schema::{ChangeEvent, UsageSite};
use napi::bindgen_prelude::*;
use napi_derive::napi;

fn json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| Error::from_reason(e.to_string()))
}

fn parse<T: for<'de> serde::Deserialize<'de>>(s: &str, what: &str) -> Result<T> {
    serde_json::from_str(s).map_err(|e| Error::from_reason(format!("parsing {what}: {e}")))
}

fn providers(json: Option<String>) -> Result<ProviderSet> {
    match json {
        Some(s) => Ok(ProviderSet::with_extra(parse(&s, "providers")?)),
        None => Ok(ProviderSet::default()),
    }
}

/// Scans one source string. Returns a JSON array of UsageSites.
#[napi]
pub fn scan_source(source: String, file: String, providers_json: Option<String>) -> Result<String> {
    let sites = firstwatch_scan::scan_source(&source, &file, &providers(providers_json)?)
        .map_err(|e| Error::from_reason(e.to_string()))?;
    json(&sites)
}

/// Scans a directory tree. Returns a JSON array of UsageSites.
#[napi]
pub fn scan_dir(path: String, providers_json: Option<String>) -> Result<String> {
    let sites = firstwatch_scan::scan_dir(Path::new(&path), &providers(providers_json)?)
        .map_err(|e| Error::from_reason(e.to_string()))?;
    json(&sites)
}

/// Joins ChangeEvents against UsageSites. JSON arrays in, JSON array of Impacts out.
#[napi]
pub fn match_impacts(events_json: String, sites_json: String) -> Result<String> {
    let events: Vec<ChangeEvent> = parse(&events_json, "events")?;
    let sites: Vec<UsageSite> = parse(&sites_json, "sites")?;
    json(&firstwatch_match::match_impacts(&events, &sites))
}

/// Diffs two OpenAPI documents. Returns a JSON array of ChangeEvents.
#[napi]
pub fn diff_openapi(old_json: String, new_json: String, provider: String) -> Result<String> {
    let old = parse(&old_json, "old spec")?;
    let new = parse(&new_json, "new spec")?;
    let events = firstwatch_watch::diff_openapi(&old, &new, &provider)
        .map_err(|e| Error::from_reason(e.to_string()))?;
    json(&events)
}
