//! Conventions module: paths, environment variables, schema helpers.
//!
//! This module encodes the workspace conventions documented in
//! `docs/conventions.md`. Every bead must use these helpers instead of
//! hard-coding paths or env var names.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Runtime output root: `target/bevy_swarm/`
pub fn output_root() -> PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target"));
    target.join("bevy_swarm")
}

/// Run reports: `target/bevy_swarm/runs/`
pub fn runs_dir() -> PathBuf {
    output_root().join("runs")
}

/// Failure isolates: `target/bevy_swarm/failures/`
pub fn failures_dir() -> PathBuf {
    output_root().join("failures")
}

/// HTML site: `target/bevy_swarm/site/`
pub fn site_dir() -> PathBuf {
    output_root().join("site")
}

/// Bisect worktrees: `target/bevy_swarm/bisect/`
pub fn bisect_dir() -> PathBuf {
    output_root().join("bisect")
}

/// Coverage index: `target/bevy_swarm/coverage-index.json`
pub fn coverage_index_path() -> PathBuf {
    output_root().join("coverage-index.json")
}

/// Special-build directory: `target/bevy_swarm-build/`
pub fn special_build_root() -> PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target"));
    target.join("bevy_swarm-build")
}

/// Persisted files root: `tests/swarm/`
pub fn persisted_root() -> PathBuf {
    PathBuf::from("tests/swarm")
}

/// Config: `tests/swarm/config.json`
pub fn config_path() -> PathBuf {
    persisted_root().join("config.json")
}

/// Known issues: `tests/swarm/issues.json`
pub fn issues_path() -> PathBuf {
    persisted_root().join("issues.json")
}

/// Golden tests: `tests/swarm/golden/`
pub fn golden_dir() -> PathBuf {
    persisted_root().join("golden")
}

/// Regression scenarios: `tests/swarm/regressions/`
pub fn regressions_dir() -> PathBuf {
    persisted_root().join("regressions")
}

/// Semantic model: `tests/swarm/model.json`
pub fn model_path() -> PathBuf {
    persisted_root().join("model.json")
}

/// Performance baselines: `tests/swarm/perf-baseline.json`
pub fn perf_baseline_path() -> PathBuf {
    persisted_root().join("perf-baseline.json")
}

/// Environment variable controlling write behavior.
///
/// Valid values: `golden,issues,model,perf,regressions` or `all`.
/// Without this env var, no persisted files are written.
pub const ENV_UPDATE: &str = "BEVY_SWARM_UPDATE";

/// Parse `BEVY_SWARM_UPDATE` into an allow-list.
///
/// Returns `None` if the env var is unset (no writes allowed).
/// Returns `UpdateMode::All` if set to `all`.
/// Otherwise returns a list of specific categories to update.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateMode {
    All,
    Categories(Vec<String>),
}

pub fn parse_update_mode() -> Option<UpdateMode> {
    let val = std::env::var(ENV_UPDATE).ok()?;
    let trimmed = val.trim();
    if trimmed.eq_ignore_ascii_case("all") {
        Some(UpdateMode::All)
    } else if trimmed.is_empty() {
        None
    } else {
        let cats: Vec<String> = trimmed
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        Some(UpdateMode::Categories(cats))
    }
}

/// Check if a category is allowed for writing.
pub fn can_update(category: &str) -> bool {
    let Some(mode) = parse_update_mode() else {
        return false;
    };
    match mode {
        UpdateMode::All => true,
        UpdateMode::Categories(ref cats) => {
            let cat_lower = category.to_lowercase();
            cats.contains(&cat_lower)
        }
    }
}

/// Schema version helper: current version is 1 (until v0.2 release).
pub const SCHEMA_VERSION: u32 = 1;

/// ISO-8601 UTC timestamp for a SystemTime (no chrono dep).
pub fn iso_timestamp(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Generate a run ID: `<UTC_yyyyMMddTHHmmss>-<8hex-of-scenario+seed-hash+nano>`
/// Unique even for back-to-back autotests (includes nanosecond suffix).
pub fn run_id(scenario_hash: u64, seed: u64) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let (year, month, day, hour, minute, second) = utc_now_components();
    // Uniqueness: process counter + pid + subsec nanos. Back-to-back
    // autotests within the same second still get distinct suffixes.
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let mix = (scenario_hash as u32) ^ (seed as u32) ^ count ^ nanos ^ pid;
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}-{mix:08x}")
}

/// Current UTC calendar time, computed without a chrono dependency
/// (Howard Hinnant's `civil_from_days` algorithm).
fn utc_now_components() -> (i64, u32, u32, u32, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    (
        year,
        month,
        day,
        (secs_of_day / 3600) as u32,
        ((secs_of_day % 3600) / 60) as u32,
        (secs_of_day % 60) as u32,
    )
}

/// Convert days since the Unix epoch to a (year, month, day) civil date.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Env-var tests mutate process-global state and must not run
    /// concurrently with each other.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_output_root_defaults_to_target() {
        // FX7: hermetic — unset CARGO_TARGET_DIR for the duration so
        // an externally-set var (CI, nextest) can't leak in.
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = std::env::var("CARGO_TARGET_DIR").ok();
        std::env::remove_var("CARGO_TARGET_DIR");
        let root = output_root();
        assert!(root.ends_with("target/bevy_swarm"));
        if let Some(s) = saved {
            std::env::set_var("CARGO_TARGET_DIR", s);
        }
    }

    #[test]
    fn test_output_root_respects_target_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = std::env::var("CARGO_TARGET_DIR").ok();
        std::env::set_var("CARGO_TARGET_DIR", "/custom/out");
        let root = output_root();
        assert_eq!(root, std::path::PathBuf::from("/custom/out/bevy_swarm"));
        match saved {
            Some(s) => std::env::set_var("CARGO_TARGET_DIR", s),
            None => std::env::remove_var("CARGO_TARGET_DIR"),
        }
    }

    #[test]
    fn test_special_build_root() {
        // FX7: hermetic (see test_output_root_defaults_to_target).
        let _guard = ENV_LOCK.lock().unwrap();
        let saved = std::env::var("CARGO_TARGET_DIR").ok();
        std::env::remove_var("CARGO_TARGET_DIR");
        let root = special_build_root();
        assert!(root.ends_with("target/bevy_swarm-build"));
        if let Some(s) = saved {
            std::env::set_var("CARGO_TARGET_DIR", s);
        }
    }

    #[test]
    fn test_parse_update_mode_all() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var(ENV_UPDATE, "all");
        assert_eq!(parse_update_mode(), Some(UpdateMode::All));
        std::env::remove_var(ENV_UPDATE);
    }

    #[test]
    fn test_parse_update_mode_categories() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var(ENV_UPDATE, "golden,issues");
        let mode = parse_update_mode();
        assert_eq!(
            mode,
            Some(UpdateMode::Categories(vec![
                "golden".into(),
                "issues".into()
            ]))
        );
        std::env::remove_var(ENV_UPDATE);
    }

    #[test]
    fn test_parse_update_mode_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var(ENV_UPDATE);
        assert_eq!(parse_update_mode(), None);
    }

    #[test]
    fn test_can_update_respects_mode() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var(ENV_UPDATE, "golden");
        assert!(can_update("golden"));
        assert!(!can_update("issues"));
        std::env::remove_var(ENV_UPDATE);
    }

    #[test]
    fn test_run_id_format() {
        let id = run_id(0xDEADBEEF, 0);
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 2);
        // Suffix is 8 hex chars (hash + nanos mixed)
        assert_eq!(parts[1].len(), 8);
        assert!(parts[1].chars().all(|c| c.is_ascii_hexdigit()));
        // Timestamp should match YYYYMMDDTHHMMSS
        assert_eq!(parts[0].len(), 15);
        assert_eq!(parts[0].chars().nth(8), Some('T'));
    }
}
