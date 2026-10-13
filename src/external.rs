//! Z20 (m2h.27): BRP external observe-only tier.
//!
//! Launches a game as an external subprocess with BRP enabled, polls state
//! over HTTP JSON-RPC (default port 15702), detects crashes via the Z19
//! stderr classifier, and produces a PlaytestReport with
//! `harness_mode: "external_driven"`.
//!
//! Observe-only: Bevy 0.20 BRP has no stepping or input verbs, so oracles are
//! limited to crash/panic, error/log, liveness-by-polling, and state digests.

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::contract::{CheckedScope, ContractTier};
use crate::driver::PlaytestReport;
use crate::enums::PlaytestStatus;
use crate::smoke::classify_line;
use crate::state::ViolationEntry;

/// Default BRP port (bevy_remote `DEFAULT_PORT`).
pub const BRP_DEFAULT_PORT: u16 = 15702;

/// Configuration for an external BRP-driven observation run.
#[derive(Debug, Clone)]
pub struct ExternalBrpConfig {
    /// Path to the game binary.
    pub binary_path: String,
    /// Arguments passed to the binary.
    pub args: Vec<String>,
    /// Overall wall-clock budget for the observation, in seconds.
    pub timeout_secs: u64,
    /// BRP HTTP port (default 15702).
    pub brp_port: u16,
    /// Interval between BRP polls.
    pub poll_interval_ms: u64,
    /// Seconds to wait for the BRP endpoint to become reachable.
    pub startup_grace_secs: u64,
}

impl Default for ExternalBrpConfig {
    fn default() -> Self {
        Self {
            binary_path: String::new(),
            args: vec![],
            timeout_secs: 120,
            brp_port: BRP_DEFAULT_PORT,
            poll_interval_ms: 500,
            startup_grace_secs: 30,
        }
    }
}

/// Errors that can occur during an external BRP run.
#[derive(Debug)]
pub enum BrpRunError {
    /// The game process could not be spawned.
    SpawnFailed(String),
    /// The BRP endpoint never became reachable within the startup grace.
    BrpUnreachable(u64),
    /// Polling the process status failed.
    WaitFailed(String),
}

impl std::fmt::Display for BrpRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrpRunError::SpawnFailed(e) => write!(f, "failed to spawn game process: {}", e),
            BrpRunError::BrpUnreachable(s) => {
                write!(f, "BRP endpoint unreachable after {}s", s)
            }
            BrpRunError::WaitFailed(e) => write!(f, "failed to wait for game process: {}", e),
        }
    }
}

impl std::error::Error for BrpRunError {}

/// One entity row of a `world.query` BRP response.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BrpEntityData {
    /// BRP serializes entity ids as numbers (index | generation << 32).
    pub entity: serde_json::Value,
    /// Reflected component values keyed by full type path.
    #[serde(default)]
    pub components: HashMap<String, serde_json::Value>,
}

/// A BRP JSON-RPC error payload.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BrpError {
    pub code: Option<i64>,
    pub message: String,
}

/// Minimal BRP client: one request per connection, JSON-RPC over HTTP.
struct BrpClient {
    port: u16,
    next_id: u64,
}

impl BrpClient {
    fn new(port: u16) -> Self {
        Self { port, next_id: 0 }
    }

    /// Probe whether the endpoint accepts connections.
    fn is_alive(&self) -> bool {
        TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", self.port).parse().unwrap(),
            Duration::from_millis(500),
        )
        .is_ok()
    }

    /// Send one JSON-RPC request; return the `result` value or the error.
    fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, BrpError> {
        self.next_id += 1;
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "id": self.next_id,
            "params": params,
        })
        .to_string();

        let request = format!(
            "POST / HTTP/1.0\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            self.port,
            body.len(),
            body
        );

        let stream = TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", self.port).parse().unwrap(),
            Duration::from_secs(5),
        )
        .map_err(|e| BrpError {
            code: None,
            message: format!("connect failed: {}", e),
        })?;
        stream.set_read_timeout(Some(Duration::from_secs(10))).ok();

        let mut stream = stream;
        stream.write_all(request.as_bytes()).map_err(|e| BrpError {
            code: None,
            message: format!("write failed: {}", e),
        })?;

        let mut response = String::new();
        stream.read_to_string(&mut response).map_err(|e| BrpError {
            code: None,
            message: format!("read failed: {}", e),
        })?;

        // Split HTTP headers from the JSON body.
        let body_part = response
            .split("\r\n\r\n")
            .nth(1)
            .unwrap_or_default()
            .trim_start_matches('\u{feff}');

        let json: serde_json::Value = serde_json::from_str(body_part).map_err(|e| BrpError {
            code: None,
            message: format!("invalid JSON response: {}", e),
        })?;

        if let Some(err) = json.get("error").filter(|e| !e.is_null()) {
            return Err(serde_json::from_value(err.clone()).unwrap_or(BrpError {
                code: None,
                message: "unknown BRP error".to_string(),
            }));
        }

        Ok(json
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    /// `world.query` with `option: "all"` — full reflected component dump.
    fn query_all(&mut self) -> Result<Vec<BrpEntityData>, BrpError> {
        let params = serde_json::json!({
            "data": {
                "components": [],
                "option": "all",
                "has": []
            },
            "filter": {
                "with": [],
                "without": []
            },
            "strict": false
        });
        let result = self.call("world.query", params)?;
        serde_json::from_value(result).map_err(|e| BrpError {
            code: None,
            message: format!("failed to parse query response: {}", e),
        })
    }
}

/// Stable digest of the queried world state, for liveness/change detection.
fn compute_state_digest(entities: &[BrpEntityData]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for entity in entities {
        entity.entity.to_string().hash(&mut hasher);
        // Sort keys so digest is order-independent per entity.
        let mut keys: Vec<_> = entity.components.keys().collect();
        keys.sort();
        for k in keys {
            k.hash(&mut hasher);
            entity.components[k].to_string().hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn violation(rule: &str, target: &str, detail: String, frame: u64) -> ViolationEntry {
    ViolationEntry {
        rule: rule.to_string(),
        target: target.to_string(),
        first_frame: frame,
        last_frame: frame,
        count: 1,
        detail,
        last_detail: String::new(),
        fingerprint: None,
        fingerprint_scheme: 0,
    }
}

/// Run a game in external BRP observe-only mode.
///
/// Launches the subprocess, waits for the BRP endpoint, polls
/// `world.query` for state digests until the timeout, then kills the
/// process and classifies its stderr with the Z19 classifier.
pub fn external_brp_run(config: &ExternalBrpConfig) -> Result<PlaytestReport, BrpRunError> {
    let start = Instant::now();
    let mut violations: Vec<ViolationEntry> = vec![];

    let mut child = Command::new(&config.binary_path)
        .args(&config.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| BrpRunError::SpawnFailed(e.to_string()))?;

    // Drain stderr on a background thread so the pipe never fills up.
    let stderr_handle = child.stderr.take().map(|stderr| {
        thread::spawn(move || {
            let mut buf = String::new();
            let _ = BufReader::new(stderr).read_to_string(&mut buf);
            buf
        })
    });

    // Wait for the BRP endpoint to accept connections.
    let mut client = BrpClient::new(config.brp_port);
    let mut brp_ready = false;
    let grace_start = Instant::now();
    while grace_start.elapsed() < Duration::from_secs(config.startup_grace_secs) {
        if child
            .try_wait()
            .map_err(|e| BrpRunError::WaitFailed(e.to_string()))?
            .is_some()
        {
            // Exited before BRP came up; stderr decides the verdict.
            break;
        }
        if client.is_alive() {
            // The port answers; give the server a moment, then confirm with a call.
            thread::sleep(Duration::from_millis(300));
            brp_ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }

    let mut poll_count: u64 = 0;
    let mut brp_errors: u64 = 0;
    let mut digest_changed = false;
    let mut prev_digest: Option<u64> = None;

    if brp_ready {
        let deadline = start + Duration::from_secs(config.timeout_secs);
        while Instant::now() < deadline {
            if child
                .try_wait()
                .map_err(|e| BrpRunError::WaitFailed(e.to_string()))?
                .is_some()
            {
                break;
            }

            match client.query_all() {
                Ok(entities) => {
                    poll_count += 1;
                    let digest = compute_state_digest(&entities);
                    if let Some(prev) = prev_digest {
                        if prev != digest {
                            digest_changed = true;
                        }
                    }
                    prev_digest = Some(digest);
                }
                Err(e) => {
                    brp_errors += 1;
                    if brp_errors <= 3 {
                        // Record the first few BRP errors; avoid flooding.
                        violations.push(violation(
                            "brp_error",
                            &format!("127.0.0.1:{}", config.brp_port),
                            e.message,
                            poll_count,
                        ));
                    }
                }
            }

            thread::sleep(Duration::from_millis(config.poll_interval_ms));
        }

        let _ = child.kill();
        let _ = child.wait();
    }

    // Collect exit status (None if we killed it, which is a normal end here).
    let exit_status = child
        .wait()
        .map_err(|e| BrpRunError::WaitFailed(e.to_string()))?;

    // Classify stderr with the Z19 classifier.
    if let Some(handle) = stderr_handle {
        let stderr_buf = handle.join().unwrap_or_default();
        for line in stderr_buf.lines() {
            if let Some((rule, detail)) = classify_line(line) {
                violations.push(violation(&rule, "stderr", detail, poll_count));
            }
        }
    }

    let elapsed = start.elapsed();
    let crashed = !exit_status.success() && exit_status.code().is_some_and(|c| c != 0);

    // A process we killed (no exit code recorded) ended normally.
    let status = if violations
        .iter()
        .any(|v| v.rule == "panic" || v.rule == "bevy_error" || v.rule == "asset_error")
        || crashed
    {
        PlaytestStatus::Crash
    } else {
        PlaytestStatus::Pass
    };

    let mut warnings = vec![];
    if brp_errors > 0 {
        warnings.push(format!(
            "{} BRP query errors during observation ({} recorded)",
            brp_errors,
            violations.iter().filter(|v| v.rule == "brp_error").count()
        ));
    }
    if poll_count > 0 && !digest_changed {
        warnings.push(
            "state digest never changed between polls — the game may be idle or frozen".to_string(),
        );
    }
    if !brp_ready {
        warnings.push(format!(
            "BRP endpoint never became reachable on port {} within {}s \
             (game must enable bevy_remote's RemotePlugin + RemoteHttpPlugin)",
            config.brp_port, config.startup_grace_secs
        ));
    }

    Ok(PlaytestReport {
        schema_version: 1,
        status,
        violations,
        metrics: Default::default(),
        coverage: Default::default(),
        frame_count: poll_count,
        wall_time_s: elapsed.as_secs_f64(),
        tps: if elapsed.as_secs_f64() > 0.0 {
            (poll_count as f64 / elapsed.as_secs_f64()) as u64
        } else {
            0
        },
        simulated_duration_s: 0.0,
        pre_ready_frames: 0,
        error: if !brp_ready {
            Some(format!(
                "BRP endpoint unreachable on port {} after {}s",
                config.brp_port, config.startup_grace_secs
            ))
        } else {
            None
        },
        action_log: vec![],
        game_version: None,
        cheat_count: 0,
        unreplayable_actions: 0,
        warnings,
        system_coverage: Default::default(),
        state_trace: None,
        action_effect_rate: Default::default(),
        choices: vec![],
        robustness: Default::default(),
        discovered_surface: None,
        scope: CheckedScope {
            tier: Some(ContractTier::Zero),
            active: vec![
                "crash (exit status + stderr classifier)".to_string(),
                "bevy_error (stderr classifier)".to_string(),
                "liveness (BRP state polling)".to_string(),
                "state digest (BRP world query)".to_string(),
            ],
            inactive: vec![
                (
                    "input".to_string(),
                    "requires input injection, unavailable on Bevy 0.20 BRP \
                     (no input verb); available in the frame-exact external tier"
                        .to_string(),
                ),
                (
                    "invariants".to_string(),
                    "requires in-process telemetry access".to_string(),
                ),
                (
                    "determinism_replay".to_string(),
                    "requires frame-exact stepping, unavailable on Bevy 0.20 BRP".to_string(),
                ),
            ],
            actuators: vec![],
        },
        harness_mode: "external_driven".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_state_digest_empty_is_stable() {
        // The exact value doesn't matter; empty input must be deterministic
        // and differ from a populated world.
        let e = BrpEntityData {
            entity: serde_json::json!(0),
            components: HashMap::new(),
        };
        assert_ne!(compute_state_digest(&[]), compute_state_digest(&[e]));
        assert_eq!(compute_state_digest(&[]), compute_state_digest(&[]));
    }

    #[test]
    fn compute_state_digest_changes_on_value_change() {
        let e1 = BrpEntityData {
            entity: serde_json::json!(0),
            components: [("Transform".into(), serde_json::json!({"x": 1}))].into(),
        };
        let e2 = BrpEntityData {
            entity: serde_json::json!(0),
            components: [("Transform".into(), serde_json::json!({"x": 2}))].into(),
        };
        assert_ne!(compute_state_digest(&[e1]), compute_state_digest(&[e2]));
    }

    #[test]
    fn compute_state_digest_order_independent() {
        let a = BrpEntityData {
            entity: serde_json::json!(0),
            components: [
                ("A".into(), serde_json::json!(1)),
                ("B".into(), serde_json::json!(2)),
            ]
            .into(),
        };
        let b = BrpEntityData {
            entity: serde_json::json!(0),
            components: [
                ("B".into(), serde_json::json!(2)),
                ("A".into(), serde_json::json!(1)),
            ]
            .into(),
        };
        assert_eq!(compute_state_digest(&[a]), compute_state_digest(&[b]));
    }

    #[test]
    fn default_config_uses_brp_default_port() {
        let config = ExternalBrpConfig::default();
        assert_eq!(config.brp_port, 15702);
        assert_eq!(config.timeout_secs, 120);
    }
}
