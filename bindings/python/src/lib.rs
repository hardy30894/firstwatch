use std::path::Path;

use firstwatch_scan::ProviderSet;
use firstwatch_schema::{ChangeEvent, UsageSite};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

fn json<T: serde::Serialize>(value: &T) -> PyResult<String> {
    serde_json::to_string(value).map_err(|e| PyValueError::new_err(e.to_string()))
}

fn parse<T: for<'de> serde::Deserialize<'de>>(s: &str, what: &str) -> PyResult<T> {
    serde_json::from_str(s).map_err(|e| PyValueError::new_err(format!("parsing {what}: {e}")))
}

fn providers(json: Option<String>) -> PyResult<ProviderSet> {
    match json {
        Some(s) => Ok(ProviderSet::with_extra(parse(&s, "providers")?)),
        None => Ok(ProviderSet::default()),
    }
}

/// Scans one source string. Returns a JSON array of UsageSites.
#[pyfunction]
#[pyo3(signature = (source, file, providers_json=None))]
fn scan_source(source: &str, file: &str, providers_json: Option<String>) -> PyResult<String> {
    let sites = firstwatch_scan::scan_source(source, file, &providers(providers_json)?)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    json(&sites)
}

/// Scans a directory tree. Returns a JSON array of UsageSites.
#[pyfunction]
#[pyo3(signature = (path, providers_json=None))]
fn scan_dir(path: &str, providers_json: Option<String>) -> PyResult<String> {
    let sites = firstwatch_scan::scan_dir(Path::new(path), &providers(providers_json)?)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    json(&sites)
}

/// Joins ChangeEvents against UsageSites. JSON arrays in, JSON array of Impacts out.
#[pyfunction]
fn match_impacts(events_json: &str, sites_json: &str) -> PyResult<String> {
    let events: Vec<ChangeEvent> = parse(events_json, "events")?;
    let sites: Vec<UsageSite> = parse(sites_json, "sites")?;
    json(&firstwatch_match::match_impacts(&events, &sites))
}

/// Diffs two OpenAPI documents. Returns a JSON array of ChangeEvents.
#[pyfunction]
fn diff_openapi(old_json: &str, new_json: &str, provider: &str) -> PyResult<String> {
    let old = parse(old_json, "old spec")?;
    let new = parse(new_json, "new spec")?;
    let events = firstwatch_watch::diff_openapi(&old, &new, provider)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    json(&events)
}

#[pymodule]
fn firstwatch(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(scan_source, m)?)?;
    m.add_function(wrap_pyfunction!(scan_dir, m)?)?;
    m.add_function(wrap_pyfunction!(match_impacts, m)?)?;
    m.add_function(wrap_pyfunction!(diff_openapi, m)?)?;
    Ok(())
}
