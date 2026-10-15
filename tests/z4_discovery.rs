//! Z4 (swarm-m2h.5): input-surface discovery by differential probing.

use bevy_swarm::discover::{discover_surface, ProbeConfig};
use bevy_swarm::headless::headless_app_factory;

/// Scoped FIXTURE_BUG mutation (same pattern as tests/autotest.rs).
struct FixtureBugGuard {
    prev: Option<String>,
}

impl FixtureBugGuard {
    fn set(env_bug: Option<&str>) -> Self {
        let prev = std::env::var("FIXTURE_BUG").ok();
        match env_bug {
            Some(b) => std::env::set_var("FIXTURE_BUG", b),
            None => std::env::remove_var("FIXTURE_BUG"),
        }
        Self { prev }
    }
}

impl Drop for FixtureBugGuard {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(p) => std::env::set_var("FIXTURE_BUG", p),
            None => std::env::remove_var("FIXTURE_BUG"),
        }
    }
}

fn surface_for(
    plugin_factory: impl Fn() -> fixture_walker::WalkerGamePlugin + Send + Sync + 'static,
) -> bevy_swarm::discover::DiscoveredSurface {
    discover_surface(
        headless_app_factory(plugin_factory),
        &ProbeConfig {
            observe_frames: 30,
            tps: 60,
            ..ProbeConfig::default()
        },
    )
}

/// The clean walker reads arrows + WASD (+ Enter for select). Static
/// pruning skips mouse and gamepad (no game system reads them).
#[test]
fn z4_discovers_walker_arrows() {
    let surface = surface_for(|| fixture_walker::WalkerGamePlugin);
    let keys: Vec<&str> = surface.keys.iter().map(|(k, _)| k.as_str()).collect();
    for expected in [
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
        "KeyW",
        "KeyA",
        "KeyS",
        "KeyD",
    ] {
        assert!(
            keys.contains(&expected),
            "expected key {expected} in discovered surface, got {keys:?}"
        );
    }
    // No spurious keys: walker binds nothing else.
    assert!(
        keys.iter().all(|k| {
            [
                "ArrowUp",
                "ArrowDown",
                "ArrowLeft",
                "ArrowRight",
                "KeyW",
                "KeyA",
                "KeyS",
                "KeyD",
            ]
            .contains(k)
        }),
        "spurious keys discovered: {keys:?}"
    );
    // Static pruning: keyboard-only game — mouse/gamepad classes have
    // no game readers, so no candidates were probed for them.
    assert!(
        surface.mouse_buttons.is_empty(),
        "no mouse effects expected for a keyboard-only game"
    );
    assert!(surface.gamepad_buttons.is_empty());
    assert!(
        surface.pruned_devices.iter().any(|d| d == "gamepad"),
        "gamepad should be pruned by static device readers, got {:?}",
        surface.pruned_devices
    );
}

/// Group completion: under FIXTURE_BUG=dead_left_key, ArrowLeft has no
/// effect and is dropped by discovery — but the group-completion
/// rule re-adds it so chaos keeps sampling it (dead_verb detection).
#[test]
#[serial_test::serial]
fn z4_group_completion_readds_dead_sibling() {
    let _guard = FixtureBugGuard::set(Some("dead_left_key"));
    let surface = surface_for(|| fixture_walker::WalkerGamePlugin.clone());
    let keys: Vec<&str> = surface.keys.iter().map(|(k, _)| k.as_str()).collect();
    assert!(
        keys.contains(&"ArrowLeft"),
        "group completion must re-add dead ArrowLeft for chaos sampling: {keys:?}"
    );
    // The re-added entry is a group-completion filler (delta 0), and
    // the live siblings have real effects.
    let left = surface
        .keys
        .iter()
        .find(|(k, _)| k == "ArrowLeft")
        .expect("ArrowLeft present");
    assert_eq!(left.1.first_divergence_frame_delta, 0);
    assert!(left
        .1
        .changed
        .iter()
        .any(|c| c.contains("group-completion")));
}

/// Static pruning bounds probe count: walker (keyboard-only) probes
/// no mouse/gamepad candidates.
#[test]
fn z4_static_pruning_skips_mouse_and_gamepad() {
    let surface = surface_for(|| fixture_walker::WalkerGamePlugin);
    assert!(
        surface
            .pruned_devices
            .contains(&"mouse_buttons".to_string())
            && surface.pruned_devices.contains(&"mouse_motion".to_string())
            && surface.pruned_devices.contains(&"wheel".to_string())
            && surface.pruned_devices.contains(&"gamepad".to_string()),
        "keyboard-only game should prune mouse+gamepad, got {:?}",
        surface.pruned_devices
    );
    // Readers: the game's keyboard system appears.
    assert!(
        surface
            .readers
            .iter()
            .any(|(class, names)| class == "keyboard" && names.iter().any(|n| n.contains("walker")))
        ,
        "expected a walker keyboard system in readers, got {:?}",
        surface.readers
    );
}

/// Discovery wall time stays bounded for the full prior candidate
/// set (~50 candidates). Budget accounts for CI runners (~5x slower
/// than a dev machine) with parallel discovery tests sharing cores:
/// the bound guards against unbounded probe EXPLOSION, not exact
/// wall-clock performance.
#[test]
fn z4_discovery_is_bounded() {
    let start = std::time::Instant::now();
    let surface = surface_for(|| fixture_walker::WalkerGamePlugin);
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 150,
        "discovery took {elapsed:?}, pruned: {:?}",
        surface.pruned_devices
    );
}

/// RawSurface conversion: discovered keys + weights feed chaos
/// sampling; mouse/gamepad flags follow discovery.
#[test]
fn z4_raw_surface_from_discovered() {
    let surface = surface_for(|| fixture_walker::WalkerGamePlugin);
    let raw = bevy_swarm::scenario::RawSurface::from_discovered(&surface);
    assert!(!raw.keys.is_empty());
    assert!(!raw.mouse_buttons);
    assert!(!raw.gamepad);
    // Every discovered key has a weight >= 1.
    for k in &raw.keys {
        assert!(
            raw.weights.get(k).copied().unwrap_or(0) >= 1,
            "key {k} unweighted"
        );
    }
}

/// Z4 × dead_verb: after discovery-based chaos on the CLEAN walker,
/// no dead_verb violation (discovered keys are all live).
#[test]
#[serial_test::serial]
fn z4_autotest_clean_walker_no_dead_verb_after_discovery() {
    let sc: bevy_swarm::scenario::Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1,"input_rate_hz":10},"duration_s":6.0,"invariants":[],"deny_ambiguities":false,"single_threaded":true}"#,
    )
    .unwrap();
    let config = bevy_swarm::autotest::AutotestConfig {
        num_seeds: 1,
        parallel: 1,
        base_scenario: sc,
        ..bevy_swarm::autotest::AutotestConfig::default()
    };
    let report = bevy_swarm::autotest::autotest(fixture_walker::WalkerGamePlugin, config)
        .expect("autotest runs");
    assert!(
        !report.failures.iter().any(|f| f.rule == "dead_verb"),
        "clean walker with discovered surface must not report dead_verb: {:?}",
        report
            .failures
            .iter()
            .map(|f| f.rule.clone())
            .collect::<Vec<_>>()
    );
}

/// Z4 × dead_verb: with FIXTURE_BUG=dead_left_key, discovery-driven
/// chaos still finds the dead verb (group completion keeps
/// ArrowLeft sampled; its siblings move, so the family is live).
#[test]
#[serial_test::serial]
fn z4_autotest_finds_dead_left_key_with_discovery() {
    let _guard = FixtureBugGuard::set(Some("dead_left_key"));
    let sc: bevy_swarm::scenario::Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":1,"input_rate_hz":10},"duration_s":20.0,"invariants":[],"deny_ambiguities":false,"single_threaded":true}"#,
    )
    .unwrap();
    let config = bevy_swarm::autotest::AutotestConfig {
        num_seeds: 1,
        duration_s: 20.0,
        parallel: 1,
        base_scenario: sc,
        ..bevy_swarm::autotest::AutotestConfig::default()
    };
    let report = bevy_swarm::autotest::autotest(fixture_walker::WalkerGamePlugin, config)
        .expect("autotest runs");
    assert!(
        report.failures.iter().any(|f| f.rule == "dead_verb"),
        "dead_left_key must be found with discovered surface: {:?}",
        report
            .failures
            .iter()
            .map(|f| f.rule.clone())
            .collect::<Vec<_>>()
    );
}
