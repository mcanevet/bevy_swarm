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
///
/// `Suppressed`: the game's error handler downgraded the error to
/// Ignore severity. Recorded so downgrade-to-Ignore games cannot make
/// swarm blind to errors, but does not trigger failure by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Suppressed,
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

/// Ensure a global tracing subscriber is installed exactly once per
/// process, with our capture layer first in the stack.
///
/// The layer routes WARN/ERROR events to the run active on the
/// emitting thread (thread-local `CURRENT_RUN`). If no run is active,
/// the event is dropped (ordinary game logging, not under test).
///
/// Safe to call repeatedly; installs once. If another subscriber was
/// already set (e.g. the game's own LogPlugin beat us to it), this is a
/// no-op — the allow-list in the driver tolerates that message.
/// le3.11 (RF7): LogPlugin::custom_layer hook — installs the capture
/// layer into the game's own LogPlugin subscriber. With this hook on
/// headless_app, log capture works even when the harness runs
/// in-process: the CaptureLayer rides on top of LogPlugin's own
/// subscriber rather than racing `set_global_default`.
pub fn log_custom_layer(_app: &mut bevy::prelude::App) -> Option<bevy::log::BoxedLayer> {
    Some(Box::new(CaptureLayer))
}

pub fn ensure_global_subscriber() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        use tracing_subscriber::layer::SubscriberExt;
        let capture = CaptureLayer;
        let subscriber = tracing_subscriber::registry()
            .with(capture)
            .with(tracing_subscriber::fmt::layer());
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}

/// Tracing layer capturing WARN/ERROR events into the current run.
/// le3.11 (RF7): public so headless_app can install it via
/// LogPlugin::custom_layer.
pub struct CaptureLayer;

impl<S> tracing_subscriber::Layer<S> for CaptureLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let level = *event.metadata().level();
        let is_warn_or_error = matches!(level, tracing::Level::WARN | tracing::Level::ERROR);
        if !is_warn_or_error {
            return;
        }
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        let message = visitor.0;
        // Allow-list: expected harness noise in multi-App processes.
        if message.contains("Could not set global logger") || message.contains("already set") {
            return;
        }
        let target = event.metadata().target().to_string();
        let frame = current_frame();
        if let Some(run_id) = CURRENT_RUN.with(Cell::get) {
            let mut map = SINK.lock().unwrap_or_else(|e| e.into_inner());
            map.entry(run_id).or_default().push(Captured::Log {
                level: if level == tracing::Level::ERROR {
                    Level::ERROR
                } else {
                    Level::WARN
                },
                target,
                message,
                frame,
            });
        }
    }
}

/// Records the `message` field of a tracing event.
struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{:?}", value);
        }
    }
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
            let mut map = SINK.lock().unwrap_or_else(|e| e.into_inner());
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
///
/// Bevy's executor wraps system runs in a span named after the system
/// when the game is built with the `trace` feature; a panic inside the
/// system then unwinds with that span current on the panicking thread.
/// Without it no span is active and we return `None`.
fn thread_local_system_name() -> Option<String> {
    let span = tracing::Span::current();
    let name = span.metadata()?.name();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Capture a borrowed BevyError + context into the current run's sink.
///
/// Severity mapping: Ignore = recorded as Suppressed (not dropped);
/// Trace/Debug/Info/Warning -> Warning; Error/Panic -> Error.
pub fn capture_bevy_error(err: &BevyError, ctx: &ErrorContext) {
    let sev = err.severity();
    let severity = match sev {
        bevy::ecs::error::Severity::Ignore => Severity::Suppressed,
        bevy::ecs::error::Severity::Warning
        | bevy::ecs::error::Severity::Trace
        | bevy::ecs::error::Severity::Debug
        | bevy::ecs::error::Severity::Info => Severity::Warning,
        bevy::ecs::error::Severity::Error | bevy::ecs::error::Severity::Panic => Severity::Error,
    };
    let context = ctx.name().to_string();
    let message = err.to_string();
    let frame = current_frame();
    if let Some(run_id) = CURRENT_RUN.with(Cell::get) {
        let mut map = SINK.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(run_id).or_default().push(Captured::BevyError {
            severity,
            context: context.clone(),
            message: message.clone(),
            frame,
        });
    }
}

/// Per-run chain of the previous (game-installed) error handler, so our
/// wrapper can record and then call the game's handler. Keyed by RunId
/// (parallel runs keep separate chains; sequential apps overwrite).
static HANDLER_CHAIN: std::sync::LazyLock<Mutex<HashMap<RunId, bevy::ecs::error::ErrorHandler>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Register the previous error handler for a run; called by run_scenario
/// before installing the swarm handler.
pub fn set_previous_handler(run_id: RunId, prev: bevy::ecs::error::ErrorHandler) {
    HANDLER_CHAIN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(run_id, prev);
}

/// Remove a run's handler-chain entry (drop guard / run exit).
pub fn clear_previous_handler(run_id: RunId) {
    HANDLER_CHAIN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&run_id);
}

/// Look up the chained previous handler for the run active on this thread.
fn chained_previous() -> Option<bevy::ecs::error::ErrorHandler> {
    let run_id = CURRENT_RUN.with(Cell::get)?;
    HANDLER_CHAIN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&run_id)
        .copied()
}

/// Public wrapper for the driver's error-handler chain lookup.
pub fn chained_previous_public() -> Option<bevy::ecs::error::ErrorHandler> {
    chained_previous()
}

thread_local! {
    static CURRENT_FRAME: Cell<u64> = const { Cell::new(0) };
}

/// FX5 Z7: set the current frame number (driver sets this each tick).
pub fn set_current_frame(frame: u64) {
    CURRENT_FRAME.with(|c| c.set(frame));
}

/// Get the current frame number (driver-set thread-local).
pub(crate) fn current_frame() -> u64 {
    CURRENT_FRAME.with(Cell::get)
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
    let mut map = SINK.lock().unwrap_or_else(|e| e.into_inner());
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

    #[test]
    fn thread_local_system_name_reads_current_span() {
        // Install a subscriber so Span::current() works.
        use tracing_subscriber::layer::SubscriberExt;
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(tracing_subscriber::fmt::layer()),
        );

        // Outside any span the extractor returns None.
        assert!(thread_local_system_name().is_none());

        let span = tracing::info_span!("my_system_name");
        let _guard = span.enter();
        assert_eq!(
            thread_local_system_name().as_deref(),
            Some("my_system_name")
        );
    }

    #[test]
    fn current_frame_roundtrip() {
        set_current_frame(4242);
        assert_eq!(current_frame(), 4242);
        set_current_frame(0);
    }

    #[test]
    fn sink_locks_recover_from_poison() {
        let run_id = RunId::new();

        // Poison SINK by panicking while holding the lock.
        let sink_poisoned = std::panic::catch_unwind(|| {
            let _guard = SINK.lock().unwrap();
            panic!("poison SINK");
        });
        assert!(sink_poisoned.is_err(), "SINK lock must be poisoned");

        // Poison HANDLER_CHAIN the same way.
        let chain_poisoned = std::panic::catch_unwind(|| {
            let _guard = HANDLER_CHAIN.lock().unwrap();
            panic!("poison HANDLER_CHAIN");
        });
        assert!(chain_poisoned.is_err(), "HANDLER_CHAIN must be poisoned");

        // Previous implementation (.lock().unwrap()) would panic here.
        // Poison-recovering accessors must keep working:
        set_previous_handler(run_id, bevy::ecs::error::match_severity);
        clear_previous_handler(run_id);

        // And the sink map itself accepts writes post-poison.
        SINK.lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(run_id, Vec::new());
        assert!(SINK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&run_id));
    }

    #[test]
    fn ignore_severity_errors_recorded_as_suppressed() {
        let run_id = RunId::new();
        set_current_run(run_id);

        // Create an Ignore-severity error (downgraded by game handler).
        let ignore_err = bevy::ecs::error::BevyError::ignore("test ignore");
        let ctx = bevy::ecs::error::ErrorContext::Command {
            name: bevy::utils::DebugName::borrowed("test_command"),
        };

        capture_bevy_error(&ignore_err, &ctx);

        clear_current_run();

        // Verify it was recorded with Suppressed severity.
        let entries = SINK.lock().unwrap_or_else(|e| e.into_inner())[&run_id]
            .iter()
            .filter_map(|e| match e {
                Captured::BevyError { severity, .. } => Some(*severity),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(entries.len(), 1, "one BevyError captured");
        assert_eq!(entries[0], Severity::Suppressed);
    }
}
