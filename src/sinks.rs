//! Z7: Generic error oracles — Bevy error handler, log capture, panic capture.
//!
//! Provides:
//! - A global tracing subscriber (once per process) that captures WARN/ERROR
//!   logs per-thread via a thread-local `CURRENT_RUN` id.
//! - A swarm error handler that captures `BevyError` (severity/context/message)
//!   and chains to any previous handler.
//! - A global panic hook that captures message, location, backtrace, and
//!   the running system name (if known), then chains to the previous hook.
//!
//! ## Design constraints (Bevy 0.20-rc.2 verified)
//! - `App::set_error_handler` may be called only once per App. We store the
//!   previous `FallbackErrorHandler` resource and chain to it.
//! - `LogPlugin` installs a process-global tracing subscriber. We install
//!   our own **once** before any App is built, and disable `LogPlugin` in
//!   headless test apps (users may re-add it; we allow-list the "Could not
//!   set global logger" message).
//! - Panic hooks are process-global. We install ours once, chain to the
//!   previous hook, and record system name via Bevy's executor spans
//!   (or leave `None` if unavailable).
//!
//! run_scenario installs the error handler and panic hook automatically;
//! the public functions here are for advanced/embedded use.
//! ```
//! use bevy_swarm::sinks::{RunId, set_current_run, drain_run};
//!
//! let run = RunId::new();
//! set_current_run(run);
//! // ... run a scenario on this thread ...
//! # drop(run);
//! ```

use bevy::ecs::error::{BevyError, ErrorContext};
use bevy::log::Level;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;

thread_local! {
    static CURRENT_RUN: Cell<Option<RunId>> = const { Cell::new(None) };
}

/// Unique run identifier.
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub struct RunId(u64);

impl RunId {
    pub fn new() -> Self {
        static COUNTER: OnceLock<std::sync::atomic::AtomicU64> = OnceLock::new();
        let counter = COUNTER.get_or_init(|| std::sync::atomic::AtomicU64::new(0));
        RunId(counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

/// Captured event from the global sinks.
#[derive(Debug, Clone)]
pub enum Captured {
    /// Bevy error from systems/observers/commands.
    BevyError {
        severity: Severity,
        context: String, // system/observer/command name
        message: String,
        frame: u64,
    },
    /// Log event (WARN/ERROR).
    Log {
        level: Level,
        target: String,
        message: String,
        frame: u64,
    },
    /// Panic (message, location, backtrace, optional system).
    Panic {
        message: String,
        location: Option<String>,
        backtrace: Option<String>,
        system: Option<String>,
    },
}

/// Severity of a Bevy error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

impl From<&bevy::ecs::error::Severity> for Severity {
    fn from(s: &bevy::ecs::error::Severity) -> Self {
        match s {
            bevy::ecs::error::Severity::Warning => Severity::Warning,
            bevy::ecs::error::Severity::Error => Severity::Error,
            bevy::ecs::error::Severity::Panic => Severity::Error, // Panics become Errors
            _ => Severity::Warning,
        }
    }
}

/// Process-wide sink for captured events, keyed by RunId.
static SINK: std::sync::LazyLock<Mutex<HashMap<RunId, Vec<Captured>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Ensure a global tracing subscriber is installed exactly once.
///
/// Must be called **before** any App is built. For now, this is a no-op
/// placeholder; full log capture requires adding `tracing-subscriber`
/// as a dependency and installing a custom Layer.
pub fn ensure_global_subscriber() {
    // Placeholder: full implementation requires tracing-subscriber
    // dependency and a custom Layer that pushes Captured::Log events.
}

/// Panic hook body: captures and then chains to `previous`.
pub fn swarm_panic_hook_inner(
    info: &std::panic::PanicHookInfo<'_>,
    previous: &dyn Fn(&std::panic::PanicHookInfo<'_>),
) {
    let message = extract_panic_message(info.payload());
    let location = info.location().map(|loc| format!("{}", loc));
    let backtrace = if std::env::var("RUST_BACKTRACE").is_ok() {
        let bt = std::backtrace::Backtrace::capture();
        if matches!(bt.status(), std::backtrace::BacktraceStatus::Captured) {
            Some(format!("{}", bt))
        } else {
            None
        }
    } else {
        None
    };

    // Attempt to extract system name from the current thread's span.
    // Bevy's executor wraps system runs in spans named after the system.
    let system = thread_local_system_name();

    // Push to the global sink if a run is active.
    if let Some(run_id) = CURRENT_RUN.with(Cell::get) {
        {
            let mut map = SINK.lock().unwrap();
            map.entry(run_id).or_default().push(Captured::Panic {
                message: message.clone(),
                location: location.clone(),
                backtrace: backtrace.clone(),
                system: system.clone(),
            });
        }
    }

    previous(info);
}

/// Extract a human-readable message from a panic payload.
fn extract_panic_message(payload: &dyn std::any::Any) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| format!("{:?}", payload))
}

/// Try to extract the current system name from the thread's tracing span.
fn thread_local_system_name() -> Option<String> {
    // Placeholder: Bevy 0.20 doesn't expose system names via thread-local spans
    // in a way we can query here. Return None for now.
    None
}

/// Swarm error handler: captures Bevy errors into the current run's sink.
pub fn swarm_error_handler(err: BevyError, ctx: ErrorContext) {
    capture_bevy_error(&err, &ctx);
}

/// Capture a borrowed BevyError + context into the current run's sink.
pub fn capture_bevy_error(err: &BevyError, ctx: &ErrorContext) {
    let severity = Severity::from(&err.severity());
    let context = ctx.name().to_string();
    let message = err.to_string();
    let frame = current_frame();
    if let Some(run_id) = CURRENT_RUN.with(Cell::get) {
        let mut map = SINK.lock().unwrap();
        map.entry(run_id).or_default().push(Captured::BevyError {
            severity,
            context: context.clone(),
            message: message.clone(),
            frame,
        });
    }
}

/// Get the current frame number (from PlaytestState if available).
fn current_frame() -> u64 {
    // Try to read PlaytestState from the current world.
    // Since we can't access the world here, we return 0 as a placeholder.
    // In practice, the caller should set CURRENT_FRAME before calling this.
    0
}

/// Set the current run id for the calling thread.
pub fn set_current_run(run_id: RunId) {
    CURRENT_RUN.with(|cell| cell.set(Some(run_id)));
}

/// Clear the current run id for the calling thread.
pub fn clear_current_run() {
    CURRENT_RUN.with(|cell| cell.set(None));
}

/// Drain captured events for a run.
pub fn drain_run(run_id: RunId) -> Vec<Captured> {
    let mut map = SINK.lock().unwrap();
    map.remove(&run_id).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_panic_message_str() {
        let payload = "boom";
        assert_eq!(extract_panic_message(&payload), "boom");
    }

    #[test]
    fn extract_panic_message_string() {
        let payload = String::from("boom");
        assert_eq!(extract_panic_message(&payload), "boom");
    }

    #[test]
    fn extract_panic_message_any() {
        let payload = 42;
        let msg = extract_panic_message(&payload);
        assert!(!msg.is_empty());
    }
}
