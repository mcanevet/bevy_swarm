//! le3.36 (RF46): shared typed expectation loader — the acceptance
//! suite used to parse expectations as untyped `serde_json::Value`
//! (typo'd keys like `must_reprot` were silently ignored); only
//! fx0_gate had the deny_unknown_fields loader. One place, both suites.

use serde_json::Value;

/// Typed expectation — a typo'd key must REJECT, not silently skip.
/// Fields exist for schema validation only; readers use the Value form.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code)]
pub struct TypedExpectation {
    #[serde(default)]
    pub schema_version: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub scaffold: Option<String>,
    #[serde(default)]
    pub seeds: Option<Vec<u64>>,
    #[serde(default)]
    pub duration_s: Option<f64>,
    #[serde(default)]
    pub invariants: Option<Value>,
    #[serde(default)]
    pub must_report: Option<Vec<TypedMustReport>>,
    #[serde(default)]
    pub must_not_report: Option<Vec<TypedMustReport>>,
    #[serde(default)]
    pub liveness: Option<Value>,
    #[serde(default)]
    pub bot: Option<Value>,
    #[serde(default)]
    pub input_rate_hz: Option<u64>,
    #[serde(default)]
    pub check_determinism: Option<u64>,
    #[serde(default)]
    pub pending: Option<Value>,
    /// Regressions carry arbitrary metadata — allowed explicitly.
    #[serde(default)]
    pub metadata: Option<Value>,
    /// le3.37 (RF47): opt-in golden-trajectory comparison. The
    /// expectation's compare_golden path is passed through to the
    /// scenario, which triggers the post-run golden check.
    #[serde(default)]
    pub compare_golden: Option<String>,
    /// Scenario name (golden entry key when compare_golden is set).
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[expect(dead_code)]
pub struct TypedMustReport {
    pub rule: String,
    #[serde(default)]
    pub detail_contains: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub min_count: Option<u64>,
}

/// Read + TYPED-validate an expectation, returning the Value form.
pub fn load_typed(raw: &str, path: &std::path::Path) -> Value {
    let typed: Result<TypedExpectation, _> = serde_json::from_str(raw);
    if let Err(e) = typed {
        panic!(
            "{}: expectation schema violation (unknown or mistyped key?): {}",
            path.display(),
            e
        );
    }
    serde_json::from_str(raw).unwrap_or_else(|e| panic!("parse {}: {}", path.display(), e))
}
