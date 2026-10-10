//! FX9: Comprehensive test suite for Z11 golden trajectory remediation.
//!
//! Each test proves a specific bug fix with mutation testing:
//! - Test fails WITHOUT the fix
//! - Test passes WITH the fix

use bevy::prelude::*;
use bevy_swarm::contract::TestApi;
use bevy_swarm::golden::{
    check_golden, check_golden_opts, record_golden, GoldenOutcome, GoldenSet, Tolerance,
};
use bevy_swarm::headless::headless_app;
use bevy_swarm::scenario::Scenario;
use std::fs;
use std::path::PathBuf;

#[derive(Resource, Clone, Copy, Default)]
struct CounterResource(u32);

/// Logic-change knob: increments counter by `speed` per frame
/// (speed=2 is a logic change → diverges from a speed=1 golden).
#[derive(Resource, Clone, Copy)]
struct Speed(u32);

fn speed_system(mut counter: ResMut<CounterResource>, speed: Option<Res<Speed>>) {
    counter.0 += speed.map_or(1, |s| s.0);
}

/// Enables the nondeterministic perturbation in nondet_system.
#[derive(Resource, Clone, Copy)]
struct NondeterminismFlag(bool);

/// Per-instance random seed injected at App construction.
#[derive(Resource, Clone, Copy)]
struct NondetSeed(pub u64);

fn nondet_system(
    mut counter: ResMut<CounterResource>,
    flag: Option<Res<NondeterminismFlag>>,
    seed: Option<Res<NondetSeed>>,
) {
    counter.0 += 1;
    // Inject perturbation when flag is enabled. Uses the per-instance
    // seed so two runs of the same scenario produce different traces.
    if flag.is_some_and(|f| f.0) {
        if let Some(seed) = seed {
            counter.0 += (seed.0 % 9973) as u32;
        }
    }
}

fn react_to_intents(
    mut counter: ResMut<CounterResource>,
    mut intents: MessageReader<bevy_swarm::contract::UserIntent>,
) {
    use bevy_swarm::contract::UserIntent;
    for intent in intents.read() {
        if matches!(intent, UserIntent::Choice { .. }) {
            counter.0 += 5;
        }
    }
}

fn increment_system(mut counter: ResMut<CounterResource>) {
    counter.0 += 1;
}

fn sync_counter_to_api(mut api: ResMut<TestApi>, counter: Res<CounterResource>) {
    if let Some(entry) = api.custom_numeric.get_mut("Counter.Value") {
        *entry = counter.0 as f64;
    }
}

/// Surface-ordered game: declares its IntentSurface (needed for the
/// chaos bot) and reacts to Choice intents.
#[derive(Clone)]
struct SurfaceGamePlugin {
    /// Order of the intent surface variants. Chaos sampling depends on
    /// this order; replayed stored inputs do not.
    reversed_surface: bool,
}

impl Plugin for SurfaceGamePlugin {
    fn build(&self, app: &mut App) {
        use bevy_swarm::contract::{IntentSurface, SurfaceVariant};
        let surface = if self.reversed_surface {
            vec![
                SurfaceVariant::Wait,
                SurfaceVariant::Choice(3),
                SurfaceVariant::Move,
            ]
        } else {
            vec![
                SurfaceVariant::Move,
                SurfaceVariant::Choice(3),
                SurfaceVariant::Wait,
            ]
        };
        app.insert_resource(IntentSurface::new(surface));
        app.init_resource::<CounterResource>();
        app.add_systems(Update, (increment_system, react_to_intents));
        app.add_systems(Last, sync_counter_to_api);
        app.add_plugins(bevy_swarm::driver::PlaytestPlugin);
        app.insert_resource(TestApi {
            score: 0,
            active_players: 0,
            custom_numeric: [(String::from("Counter.Value"), 0.0)].into_iter().collect(),
            custom_text: Default::default(),
        });
    }
}

/// Nondeterministic counter game: when `inject` is true, a hidden
/// nondeterministic perturbation based on a per-instance random seed
/// changes the counter. Each App construction generates a fresh seed,
/// ensuring two runs of the same scenario produce different traces.
/// Also supports a `speed` multiplier.
#[derive(Clone)]
struct NondetGamePlugin {
    inject: bool,
    speed: u32,
}

impl Plugin for NondetGamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CounterResource>();
        app.insert_resource(NondeterminismFlag(self.inject));
        // Per-instance random seed for nondeterminism (immune to
        // global-state races under parallel tests).
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        app.insert_resource(NondetSeed(seed));
        if self.speed != 1 {
            app.insert_resource(Speed(self.speed));
        }
        app.add_systems(Update, (speed_system, nondet_system));
        app.add_systems(Last, sync_counter_to_api);
        app.add_plugins(bevy_swarm::driver::PlaytestPlugin);
        app.insert_resource(TestApi {
            score: 0,
            active_players: 0,
            custom_numeric: [(String::from("Counter.Value"), 0.0)].into_iter().collect(),
            custom_text: Default::default(),
        });
    }
}

fn nondet_app(flag: bool) -> impl Fn() -> App {
    move || {
        headless_app(NondetGamePlugin {
            inject: flag,
            speed: 1,
        })()
    }
}

fn speedy_app(speed: u32) -> impl Fn() -> App {
    move || {
        headless_app(NondetGamePlugin {
            inject: false,
            speed,
        })()
    }
}

#[derive(Clone, Default)]
struct CounterGamePlugin;

impl Plugin for CounterGamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CounterResource>();
        app.add_systems(Update, increment_system);
        app.add_systems(Last, sync_counter_to_api);
        app.add_plugins(bevy_swarm::driver::PlaytestPlugin);
        app.insert_resource(TestApi {
            score: 0,
            active_players: 0,
            custom_numeric: [(String::from("Counter.Value"), 0.0)].into_iter().collect(),
            custom_text: Default::default(),
        });
    }
}

fn counter_app() -> App {
    headless_app(CounterGamePlugin)()
}

fn surface_game(reversed_surface: bool) -> impl Fn() -> App {
    move || headless_app(SurfaceGamePlugin { reversed_surface })()
}

fn temp_fx9_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bevy_swarm_fx9_{tag}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn chaos_scenario(seed: u64) -> Scenario {
    serde_json::from_str(&format!(
        r#"{{"name":"chaos","bot":{{"type":"chaos","seed":{},"input_rate_hz":100}},"duration_s":0.1}}"#,
        seed
    ))
    .unwrap()
}

/// ITEM 1: Per-interval snapshots DURING the run
///
/// BUG: Snapshots taken AFTER run from final world state.
/// FIX: Take per-interval snapshots during the run with scenario TPS.
/// TEST: Scenario with incrementing counter → snapshots must show strictly increasing values.
#[test]
fn fx9_item1_snapshots_show_increasing_values() {
    let dir = temp_fx9_dir("item1_snapshots");
    let scenario = chaos_scenario(123);

    // Record golden (should capture per-interval snapshots)
    let golden_set = record_golden(counter_app, std::slice::from_ref(&scenario), &dir).unwrap();

    // Find the recorded golden file
    let (_, _golden_file) = &golden_set.scenarios.first().unwrap();
    let golden_path = dir.join(format!("scenario-chaos-{}.golden.json", scenario.bot.seed));
    let json = fs::read_to_string(&golden_path).unwrap();
    let golden: bevy_swarm::golden::GoldenFile = serde_json::from_str(&json).unwrap();

    // CRITICAL: Snapshots must show strictly increasing counter values
    // If all snapshots are the same (final state), this fails
    assert!(
        golden.snapshots.len() >= 2,
        "Should have at least 2 snapshots, got {}",
        golden.snapshots.len()
    );

    // Check that snapshots show increasing values (not all identical)
    let mut prev_value: Option<u32> = None;
    for snap in &golden.snapshots {
        let counter_val = snap
            .state
            .get("TestApi.Counter.Value")
            .and_then(|v| v.parse::<u32>().ok())
            .expect("Counter.Value should be in snapshot");

        if let Some(prev) = prev_value {
            // Counter increments every frame, so later snapshots must have higher values
            assert!(
                counter_val > prev,
                "Snapshot at frame {} has value {} which is not > previous value {} \
                 (indicates all snapshots taken from final state)",
                snap.frame,
                counter_val,
                prev
            );
        }
        prev_value = Some(counter_val);
    }
}

/// ITEM 2: Replay stored inputs
///
/// BUG: check_golden runs chaos bot again instead of replaying stored inputs.
/// FIX: Use BotType::Replay or feed stored inputs via ChoiceStream/ReplayIntent.
/// TEST: Change IntentSurface order → replay still matches golden because it uses stored inputs.
#[test]
fn fx9_item2_replay_uses_stored_inputs_not_chaos() {
    let dir = temp_fx9_dir("item2_replay");

    // Record with chaos bot against a game declaring an IntentSurface.
    let scenario = chaos_scenario(456);
    let recorded =
        record_golden(surface_game(false), std::slice::from_ref(&scenario), &dir).unwrap();

    // Load golden to verify inputs were stored.
    let golden_path = dir.join(format!("scenario-chaos-{}.golden.json", scenario.bot.seed));
    let json = fs::read_to_string(&golden_path).unwrap();
    let golden: bevy_swarm::golden::GoldenFile = serde_json::from_str(&json).unwrap();

    // FIX ITEM 2: GoldenFile.inputs should now contain structured
    // ReplayInput entries (not just strings), enabling proper replay.
    assert!(
        !golden.inputs.is_empty(),
        "Recorded golden must have stored inputs for replay"
    );

    // STRONG assertion per the bead: replay uses STORED inputs, so a
    // reordered IntentSurface must NOT change the check outcome —
    // chaos would resample differently, replay repeats the recording.
    let set = GoldenSet {
        scenarios: recorded.scenarios.clone(),
        dir: dir.clone(),
    };
    let report = check_golden(surface_game(true), &set, &Tolerance::default()).unwrap();
    assert!(
        report.all_same(),
        "Replay must use stored inputs: reordered IntentSurface must not change the golden comparison: {:?}",
        report.per_scenario
    );

    // Mutation substrate: if the fix regressed to chaos, the
    // check_golden would resample and likely diverge from the recorded
    // golden (especially with multiple runs or different surfaces).
}

/// ITEM 3: Diff shows new values + tolerances
///
/// BUG: Diff lists only old values with empty "new"; abs_tol/rel_tol unused.
/// FIX: Populate both old/new in diff output, apply tolerances (diff_states).
/// TEST: Float change within tol passes, outside fails with both values shown.
#[test]
fn fx9_item3_tolerance_applied_and_both_values_shown() {
    use bevy_swarm::golden::diff_states;

    let tol = Tolerance {
        abs_tol: 0.1,
        rel_tol: 0.1,
        ignore_patterns: vec![],
    };

    let mut old = std::collections::BTreeMap::new();
    old.insert("float.within".to_string(), "1.0".to_string());
    old.insert("float.outside".to_string(), "2.0".to_string());
    old.insert("text.same".to_string(), "same".to_string());

    let mut new = std::collections::BTreeMap::new();
    new.insert("float.within".to_string(), "1.05".to_string()); // within tol
    new.insert("float.outside".to_string(), "3.0".to_string()); // outside tol
    new.insert("text.same".to_string(), "same".to_string());

    let changed = diff_states(&old, &new, &tol);

    // Within-tolerance float must NOT be reported
    assert!(
        !changed.iter().any(|(k, _, _)| k == "float.within"),
        "within-tolerance change must be filtered: {changed:?}"
    );
    // Matching values must not be reported
    assert!(
        !changed.iter().any(|(k, _, _)| k == "text.same"),
        "matching values must not be reported: {changed:?}"
    );
    // Outside-tolerance float must appear with BOTH values
    let entry = changed
        .iter()
        .find(|(k, _, _)| k == "float.outside")
        .expect("outside-tolerance change must be reported");
    assert_eq!(entry.1, "2.0", "diff must show the OLD value");
    assert_eq!(
        entry.2, "3.0",
        "diff must show the NEW value (was empty before fix)"
    );
}

/// ITEM 4: Determinism pre-check
///
/// BUG: No determinism pre-check before comparing to golden.
/// FIX: check_golden runs the scenario twice; on run-vs-run mismatch
/// the outcome summary says "nondeterministic", not golden divergence.
/// TEST: Inject nondeterminism → report says nondeterministic.
#[test]
fn fx9_item4_determinism_precheck_before_golden_compare() {
    let dir = temp_fx9_dir("item4_nondet");
    let scenario = chaos_scenario(789);

    // Record golden against the deterministic variant (inject=false).
    record_golden(nondet_app(false), std::slice::from_ref(&scenario), &dir).unwrap();

    // Sanity: deterministic game matches its golden.
    let set = GoldenSet {
        scenarios: vec![(
            format!("scenario-chaos-{}", scenario.bot.seed),
            scenario.clone(),
        )],
        dir: dir.clone(),
    };
    let det_report = check_golden(nondet_app(false), &set, &Tolerance::default()).unwrap();
    assert!(det_report.all_same(), "{:?}", det_report.per_scenario);

    // Nondeterministic game (inject=true): the two internal runs
    // disagree → outcome must report "nondeterministic".
    let report = check_golden(nondet_app(true), &set, &Tolerance::default()).unwrap();
    assert!(!report.all_same());
    match &report.per_scenario[0].1 {
        GoldenOutcome::Diverged { summary, .. } => {
            assert_eq!(
                summary, "nondeterministic",
                "run-vs-run mismatch must be reported as nondeterministic, not golden_diverged"
            );
        }
        other => panic!("expected Diverged, got {other:?}"),
    }
}

/// ITEM 5: BEVY_SWARM_UPDATE=golden rewrite
///
/// BUG: Just sets env var, doesn't actually rewrite golden file.
/// FIX: check_golden rewrites the golden file when update mode is enabled.
/// TEST: Divergent game → force-update rewrites golden with new digests.
#[test]
fn fx9_item5_update_mode_rewrites_golden() {
    let dir = temp_fx9_dir("item5_update");
    let scenario = chaos_scenario(999);

    // Record golden
    record_golden(counter_app, std::slice::from_ref(&scenario), &dir).unwrap();

    let golden_path = dir.join(format!("scenario-chaos-{}.golden.json", scenario.bot.seed));
    let initial_digests = {
        let json = fs::read_to_string(&golden_path).unwrap();
        let golden: bevy_swarm::golden::GoldenFile = serde_json::from_str(&json).unwrap();
        golden.digests.clone()
    };

    // Run check with the CHANGED game (speed=2 → diverges) and
    // force_update=true (stands in for BEVY_SWARM_UPDATE=golden,
    // without the env-var race between parallel tests).
    let set = GoldenSet {
        scenarios: vec![(
            format!("scenario-chaos-{}", scenario.bot.seed),
            scenario.clone(),
        )],
        dir: dir.clone(),
    };
    let report = check_golden_opts(speedy_app(2), &set, &Tolerance::default(), true).unwrap();
    assert!(!report.all_same(), "logic change must diverge");

    // The golden file must now hold the NEW digests (rewritten).
    let json = fs::read_to_string(&golden_path).unwrap();
    let rewritten: bevy_swarm::golden::GoldenFile = serde_json::from_str(&json).unwrap();
    assert_ne!(
        rewritten.digests, initial_digests,
        "update mode must rewrite the golden with the diverged (current) digests"
    );

    // Proof of correctness: the updated golden now matches the new game.
    let recheck = check_golden(speedy_app(2), &set, &Tolerance::default()).unwrap();
    assert!(
        recheck.all_same(),
        "after rewrite, the new game must match the updated golden: {:?}",
        recheck.per_scenario
    );
}

/// ITEM 6: save/load round-trip
///
/// BUG: save() returns Err (was previously wiping digests).
/// FIX: load→save→load preserves digests/inputs/snapshots.
/// TEST: Round-trip test asserts file-content equality.
#[test]
fn fx9_item6_save_load_roundtrip() {
    let dir = temp_fx9_dir("item6_roundtrip");
    let scenario = chaos_scenario(111);

    // Record golden
    record_golden(counter_app, std::slice::from_ref(&scenario), &dir).unwrap();

    let name = format!("scenario-chaos-{}", scenario.bot.seed);
    let golden_path = dir.join(format!("{name}.golden.json"));
    let before_json = fs::read_to_string(&golden_path).unwrap();

    // load → save → load must preserve everything on disk.
    let set1 = GoldenSet::load(&dir).unwrap();
    set1.save().unwrap();
    let set2 = GoldenSet::load(&dir).unwrap();
    set2.save().unwrap();

    let after_json = fs::read_to_string(&golden_path).unwrap();
    assert_eq!(
        before_json, after_json,
        "load→save must not alter golden files (digests/inputs/snapshots preserved)"
    );

    // save() must refuse to BLINDLY write goldens it cannot preserve
    // (a set referencing a non-existent golden).
    let bogus = GoldenSet {
        scenarios: vec![("never-recorded".to_string(), scenario.clone())],
        dir: dir.clone(),
    };
    assert!(
        bogus.save().is_err(),
        "save must not fabricate a golden that wipes digests"
    );
}

/// ITEM 7: all_same length-mismatch test
///
/// BUG: Digest-count mismatch sets outcome Diverged but leaves all_same true.
/// FIX: Ensure digest-count mismatch sets all_same=false.
#[test]
fn fx9_item7_length_mismatch_sets_all_same_false() {
    let dir = temp_fx9_dir("item7_length");
    let scenario = chaos_scenario(222);

    // Record golden
    record_golden(counter_app, std::slice::from_ref(&scenario), &dir).unwrap();

    // Load golden
    let golden_path = dir.join(format!("scenario-chaos-{}.golden.json", scenario.bot.seed));
    let json = fs::read_to_string(&golden_path).unwrap();
    let mut golden: bevy_swarm::golden::GoldenFile = serde_json::from_str(&json).unwrap();

    // Corrupt the golden by truncating digests
    let original_len = golden.digests.len();
    golden.digests.truncate(original_len / 2);

    let corrupted_path = dir.join("corrupted.golden.json");
    let json = serde_json::to_string_pretty(&golden).unwrap();
    fs::write(&corrupted_path, json).unwrap();

    // Create golden set pointing to corrupted file
    let set = GoldenSet {
        scenarios: vec![("corrupted".to_string(), scenario.clone())],
        dir: dir.clone(),
    };

    // Check against corrupted golden
    let report = check_golden(counter_app, &set, &Tolerance::default()).unwrap();

    // Length mismatch MUST set all_same=false
    assert!(
        !report.all_same(),
        "Length mismatch should set all_same=false"
    );

    // Should be reported as diverged
    match &report.per_scenario[0].1 {
        GoldenOutcome::Diverged { changed, .. } => {
            assert!(
                changed.iter().any(|(k, _, _)| k == "length"),
                "Should report length mismatch"
            );
        }
        other => panic!("Expected Diverged, got {:?}", other),
    }
}

/// ITEM 8: Naming by scenario name + seed
///
/// BUG: Files named scenario-<index>-<seed> overwrite on reorder.
/// FIX: Name by scenario name + seed.
/// TEST: Two scenarios with same seed but different names produce different files.
#[test]
fn fx9_item8_naming_by_name_plus_seed() {
    let dir = temp_fx9_dir("item8_naming");

    // Two scenarios with same seed but DIFFERENT NAMES
    let scenario1: Scenario = serde_json::from_str(
        r#"{"name":"scenario_one","bot":{"type":"chaos","seed":42,"input_rate_hz":100},"duration_s":0.1}"#,
    )
    .unwrap();
    let scenario2: Scenario = serde_json::from_str(
        r#"{"name":"scenario_two","bot":{"type":"chaos","seed":42,"input_rate_hz":100},"duration_s":0.1}"#,
    )
    .unwrap();

    // Record both
    record_golden(counter_app, &[scenario1.clone(), scenario2.clone()], &dir).unwrap();

    // List files
    let files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    // Should have 2 distinct files (different names prevent collision)
    assert_eq!(files.len(), 2, "Should have 2 golden files");

    // Files should be named scenario-<name>-<seed>
    assert!(
        files.iter().any(|f| f.contains("scenario_one")),
        "scenario_one file should exist"
    );
    assert!(
        files.iter().any(|f| f.contains("scenario_two")),
        "scenario_two file should exist"
    );
}

/// ITEM 9: Remove FX9 comments
///
/// BUG: Code contains // FX9: comments describing unimplemented behavior.
/// FIX: Strip all bead-status comments from golden.rs.
/// TEST: No "FX9:" bead-status comments remain in the golden module.
#[test]
fn fx9_item9_no_fx_comments_in_code() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/golden.rs"))
        .expect("golden.rs source should be readable");
    assert!(
        !src.contains("FX9"),
        "golden.rs must not carry FX9 bead-status comments:\n{}",
        src.lines()
            .filter(|l| l.contains("FX9"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
