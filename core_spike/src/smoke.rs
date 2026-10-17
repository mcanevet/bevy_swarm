//! Z19 (m2h.20): smoke runner + stderr classifier for real binaries.
//!
//! - `classify_stderr` — maps stderr lines to Z7 rule violations
//! - `smoke_run` — launches an external binary, captures output, returns PlaytestReport

use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Classify a single stderr line into a Z7 rule violation.
/// Returns (rule, detail) if the line matches a known pattern.
pub fn classify_line(line: &str) -> Option<(String, String)> {
    let line_lower = line.to_lowercase();

    // Panic patterns (Z7 panic rule)
    if line_lower.contains("panicked at") || line_lower.contains("thread 'main' panicked") {
        // Extract the panic message/location
        let detail = extract_panic_message(line);
        return Some(("panic".to_string(), detail));
    }

    // Bevy error patterns (Z7 bevy_error rule): both the raw
    // log-level prefix ("ERROR bevy_log: ...") and a tagged variant.
    if line.trim_start().starts_with("ERROR") || line_lower.contains("bevy_error:") {
        return Some(("bevy_error".to_string(), line.trim().to_string()));
    }

    // Log warn patterns (Z7 log_warn rule)
    if line_lower.contains("warn")
        && (line_lower.contains("bevy") || line_lower.contains("commandqueue"))
    {
        return Some(("log_warn".to_string(), line.trim().to_string()));
    }

    // Asset loading errors
    if line_lower.contains("asset")
        && (line_lower.contains("failed") || line_lower.contains("error"))
    {
        return Some(("asset_error".to_string(), line.trim().to_string()));
    }

    None
}

/// Extract the core panic message from a panic line.
fn extract_panic_message(line: &str) -> String {
    // Pattern: "panicked at 'MESSAGE', file.rs:LINE:COL"
    // or "panicked at file.rs:LINE:COL: MESSAGE"
    if let Some(idx) = line.find("panicked at") {
        let rest = line[idx + 11..].trim_start();
        // Try quoted message first: "panicked at 'MSG', file:LINE:COL"
        if let Some(stripped) = rest.strip_prefix('\'') {
            if let Some(end) = stripped.find('\'') {
                return stripped[..end].to_string();
            }
        }
        // Modern format: "panicked at file.rs:LINE:COL:\n MSG"
        if let Some(colon) = rest.find(':') {
            return rest[colon + 1..].trim().to_string();
        }
        return rest.to_string();
    }
    line.to_string()
}

/// Run an external binary in smoke mode: capture output, check exit status,
/// classify stderr violations.
///
/// Returns a PlaytestReport with:
/// - status: Pass if exit code 0, Crash otherwise
/// - violations: classified stderr lines
/// - harness_mode: "external_smoke"
pub fn smoke_run(
    binary_path: &std::path::Path,
    args: &[&str],
    timeout_s: u64,
) -> Result<crate::core_state::PlaytestReport, SmokeError> {
    let start = Instant::now();

    // Spawn the process
    let mut child = Command::new(binary_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| SmokeError::SpawnFailed(e.to_string()))?;

    // Wait for the process with timeout
    let result = loop {
        if start.elapsed() > Duration::from_secs(timeout_s) {
            let _ = child.kill();
            return Err(SmokeError::Timeout(timeout_s));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(SmokeError::WaitFailed(e.to_string())),
        }
    };

    let elapsed = start.elapsed();

    // Capture stdout/stderr after the process has exited (pipes are
    // then fully written and closed by the child).
    let stderr_lines: Vec<String> = child
        .stderr
        .take()
        .map(|mut s| {
            let mut buf = String::new();
            s.read_to_string(&mut buf).ok();
            buf.lines().map(|l| l.to_string()).collect()
        })
        .unwrap_or_default();
    let stdout_lines: Vec<String> = child
        .stdout
        .take()
        .map(|mut s| {
            let mut buf = String::new();
            s.read_to_string(&mut buf).ok();
            buf.lines().map(|l| l.to_string()).collect()
        })
        .unwrap_or_default();

    // Classify violations from stderr
    let violations: Vec<_> = stderr_lines
        .iter()
        .filter_map(|line| {
            classify_line(line).map(|(rule, detail)| crate::core_state::ViolationEntry {
                rule,
                target: "stderr".to_string(),
                first_frame: 0,
                last_frame: 0,
                count: 1,
                detail,
                last_detail: String::new(),
                fingerprint: None,
                fingerprint_scheme: 0,
            })
        })
        .collect();

    let status = if result.success() {
        crate::enums::PlaytestStatus::Pass
    } else {
        crate::enums::PlaytestStatus::Crash
    };

    // Build the report
    let mut report = crate::core_state::PlaytestReport {
        schema_version: 1,
        status,
        violations,
        metrics: Default::default(),
        coverage: Default::default(),
        frame_count: 0,
        wall_time_s: elapsed.as_secs_f64(),
        tps: 0,
        simulated_duration_s: 0.0,
        pre_ready_frames: 0,
        error: if result.success() {
            None
        } else {
            Some(format!("Process exited with code: {:?}", result.code()))
        },
        action_log: vec![],
        game_version: None,
        cheat_count: 0,
        unreplayable_actions: 0,
        warnings: vec![],
        system_coverage: Default::default(),
        state_trace: None,
        action_effect_rate: Default::default(),
        choices: vec![],
        robustness: Default::default(),
        discovered_surface: None,
        scope: crate::core_state::CheckedScope {
            tier: Some(crate::core_state::ContractTier::Zero),
            active: vec![
                "panic (stderr classifier)".to_string(),
                "bevy_error (stderr classifier)".to_string(),
                "crash (exit status)".to_string(),
            ],
            inactive: vec![
                (
                    "input".to_string(),
                    "requires input injection, available in the frame-exact external tier"
                        .to_string(),
                ),
                (
                    "invariants".to_string(),
                    "requires in-process telemetry access — to enable, add bevy_playtest as a dev-dependency and drive the game plugin in-process (see README 'Contract tiers')"
                        .to_string(),
                ),
                (
                    "liveness".to_string(),
                    "requires state polling, available in the BRP external tier".to_string(),
                ),
            ],
            actuators: vec![],
        },
        harness_mode: "external_smoke".to_string(),
    };

    // Add stdout as warnings if non-empty
    if !stdout_lines.is_empty() {
        report.warnings = stdout_lines;
    }

    Ok(report)
}

/// Errors that can occur during smoke runs.
#[derive(Debug)]
pub enum SmokeError {
    SpawnFailed(String),
    Timeout(u64),
    WaitFailed(String),
}

impl std::fmt::Display for SmokeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmokeError::SpawnFailed(e) => write!(f, "Failed to spawn process: {}", e),
            SmokeError::Timeout(s) => write!(f, "Process timed out after {}s", s),
            SmokeError::WaitFailed(e) => write!(f, "Failed to wait for process: {}", e),
        }
    }
}

impl std::error::Error for SmokeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_panic_line() {
        let line = "thread 'main' panicked at 'assertion failed: x > 0', src/main.rs:42:13";
        let (rule, detail) = classify_line(line).expect("should classify");
        assert_eq!(rule, "panic");
        assert!(detail.contains("assertion failed"));
    }

    #[test]
    fn classify_bevy_error() {
        let line = "ERROR bevy_log: Could not set global logger";
        let (rule, detail) = classify_line(line).expect("should classify");
        assert_eq!(rule, "bevy_error");
        assert!(detail.contains("Could not set global logger"));
    }

    #[test]
    fn classify_unknown_line() {
        let line = "INFO Starting game...";
        assert!(classify_line(line).is_none());
    }
}
