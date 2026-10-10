//! FX7: TDD for fingerprint stability fixes (swarm-qo7.7 remediation).
//!
//! Tests written FIRST; each maps to a re-review checklist item.
//! Mutations of the fixes must make the corresponding test FAIL.

use bevy_swarm::fingerprint::{config_normalizer, Normalizer};

// ---------------------------------------------------------------------------
// Item 1: merge_keys must merge ONLY messages containing the key
// ---------------------------------------------------------------------------

#[test]
fn fx7_merge_keys_only_merge_messages_containing_key() {
    let n = Normalizer {
        merge_keys: vec!["volatile_counter".to_string()],
        ..Normalizer::default()
    };

    let with_key_a = n.fingerprint(
        "violation",
        "panic",
        &["world".to_string()],
        "panic with volatile_counter=7 while idle",
    );
    let with_key_b = n.fingerprint(
        "violation",
        "panic",
        &["world".to_string()],
        "panic with volatile_counter=99 while idle",
    );
    let without_key = n.fingerprint(
        "violation",
        "panic",
        &["world".to_string()],
        "panic with bad_index while idle",
    );

    assert_eq!(
        with_key_a, with_key_b,
        "merge key present: volatile value must not split the fingerprint"
    );
    assert_ne!(
        with_key_a, without_key,
        "message WITHOUT the merge key must NOT collapse to [merged]"
    );
}

// ---------------------------------------------------------------------------
// Item 2: reject_keys must split on the key's VALUE (pre-normalization)
// ---------------------------------------------------------------------------

#[test]
fn fx7_reject_keys_split_on_value_not_key_presence() {
    // The values must normalize away (numeric: "entity_id=123" →
    // "entity_id=*") — that's the shipped bug: after normalization the
    // values are gone, so key-presence alone could never split.
    let n = Normalizer {
        reject_keys: vec!["entity_id".to_string()],
        ..Normalizer::default()
    };

    let ent_a = n.fingerprint(
        "violation",
        "leak",
        &["world".to_string()],
        "entity_id=123 leaked resource",
    );
    let ent_b = n.fingerprint(
        "violation",
        "leak",
        &["world".to_string()],
        "entity_id=456 leaked resource",
    );
    // Same value, different volatile elsewhere (normalizes away): no split.
    let ent_a2 = n.fingerprint(
        "violation",
        "leak",
        &["world".to_string()],
        "entity_id=123 leaked resource from entity:99v1",
    );
    let ent_a3 = n.fingerprint(
        "violation",
        "leak",
        &["world".to_string()],
        "entity_id=123 leaked resource from entity:07v2",
    );

    assert_ne!(
        ent_a, ent_b,
        "different reject_key VALUES must produce different fingerprints \
         even though the values themselves normalize away"
    );
    assert_eq!(
        ent_a2, ent_a3,
        "same reject_key value must NOT split despite volatile differences"
    );
}

// ---------------------------------------------------------------------------
// Item 3: config loading — unit + integration (tests/swarm/config.json)
// ---------------------------------------------------------------------------

#[test]
#[serial_test::serial]
fn fx7_config_normalizer_reads_tests_swarm_config() {
    // tests/swarm/config.json carries a "fingerprints" object (added by
    // this bead): {"reject_keys": ["channel"], "merge_keys": ["volatile"]}
    let n = config_normalizer();
    assert!(
        n.reject_keys.iter().any(|k| k == "channel"),
        "reject_keys must include 'channel' from tests/swarm/config.json: {:?}",
        n.reject_keys
    );
    assert!(
        n.merge_keys.iter().any(|k| k == "volatile"),
        "merge_keys must include 'volatile' from tests/swarm/config.json: {:?}",
        n.merge_keys
    );
}

#[test]
#[serial_test::serial]
fn fx7_config_normalizer_missing_file_falls_back_to_default() {
    // Point at a cwd where no config exists: config_normalizer reads
    // conventions::config_path() (cwd-relative "tests/swarm/config.json").
    let tmp = std::env::temp_dir().join("bevy_swarm_fx7_nocfg");
    std::fs::create_dir_all(&tmp).unwrap();
    let cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&tmp).unwrap();
    let n = config_normalizer();
    std::env::set_current_dir(cwd).unwrap();
    assert!(
        n.reject_keys.is_empty(),
        "missing config → empty reject_keys"
    );
    assert!(n.merge_keys.is_empty(), "missing config → empty merge_keys");
}

// ---------------------------------------------------------------------------
// Item 4: e2e acceptance — real fixture runs, not synthetic strings
// ---------------------------------------------------------------------------

/// walker FIXTURE_BUG=panic_on_edge over 3 seeds → exactly 1 distinct
/// fingerprint for the panic rule.
#[test]
#[serial_test::serial]
fn fx7_e2e_walker_panic_on_edge_one_fingerprint_across_seeds() {
    use bevy_swarm::branch::ScenarioRunner;
    use bevy_swarm::scenario::Scenario;

    std::env::set_var("FIXTURE_BUG", "panic_on_edge");

    let mut fps: Vec<bevy_swarm::fingerprint::Fingerprint> = vec![];
    let mut saw_panic = false;
    for seed in [1u64, 2, 3] {
        let scen: Scenario = serde_json::from_str(&format!(
            r#"{{"bot":{{"type":"chaos","seed":{seed},"input_rate_hz":10}},"duration_s":6.0,"invariants":[],"deny_ambiguities":false,"single_threaded":true}}"#
        ))
        .unwrap();
        let factory = || bevy_swarm::headless::headless_app(fixture_walker::WalkerGamePlugin)();
        let runner = bevy_swarm::branch::InProcess { factory };
        let rep = runner.run(&scen).unwrap();
        for v in &rep.violations {
            if v.rule == "panic" {
                saw_panic = true;
                if let Some(f) = &v.fingerprint {
                    fps.push(f.clone());
                }
            }
        }
    }
    std::env::remove_var("FIXTURE_BUG");

    assert!(saw_panic, "panic_on_edge must fire (seed-dependent)");
    assert!(!fps.is_empty(), "panic violations must carry fingerprints");
    let uniq: std::collections::HashSet<_> = fps.iter().collect();
    assert_eq!(
        uniq.len(),
        1,
        "one bug across 3 seeds must yield exactly 1 fingerprint, got {uniq:?}"
    );
}

/// spawner leak, counted via a custom entity-count invariant, leaking on
/// entities spawned at different rates across two seeds → 1 fingerprint.
#[test]
#[serial_test::serial]
fn fx7_e2e_spawner_leak_two_entity_counts_one_fingerprint() {
    use bevy_swarm::branch::ScenarioRunner;
    use bevy_swarm::scenario::Scenario;

    std::env::set_var("FIXTURE_BUG", "leak");

    let scen: Scenario = serde_json::from_str(
        r#"{"bot":{"type":"chaos","seed":42,"input_rate_hz":10},"duration_s":2.0,
            "invariants":[
              {"name":"entity_count_bound","rule":"custom",
               "query":{"with":["LeakyEntity"]},
               "check":"below","value":15}
            ],"deny_ambiguities":false,"single_threaded":true}"#,
    )
    .unwrap();

    let factory = || bevy_swarm::headless::headless_app(fixture_spawner::SpawnerGamePlugin)();
    let runner = bevy_swarm::branch::InProcess { factory };
    // Two seeds: the leak fires with different per-run details (frames,
    // contexts) while remaining the SAME bug — fingerprints must collapse.
    let mut leak_fps: Vec<bevy_swarm::fingerprint::Fingerprint> = vec![];
    let mut first_violations: Option<Vec<String>> = None;
    for seed in [42u64, 7] {
        let mut s = scen.clone();
        s.bot.seed = seed;
        let rep = runner.run(&s).unwrap();
        for v in &rep.violations {
            if v.rule == "entity_count_bound" {
                if let Some(f) = &v.fingerprint {
                    leak_fps.push(f.clone());
                }
            }
        }
        if first_violations.is_none() {
            first_violations = Some(
                rep.violations
                    .iter()
                    .map(|v| format!("{}: {}", v.rule, v.detail))
                    .collect(),
            );
        }
    }
    std::env::remove_var("FIXTURE_BUG");

    assert!(
        !leak_fps.is_empty(),
        "leak invariant must fire: {:?}",
        first_violations.unwrap()
    );
    let uniq: std::collections::HashSet<_> = leak_fps.iter().collect();
    assert_eq!(
        uniq.len(),
        1,
        "one leak bug → exactly 1 fingerprint, got {uniq:?}"
    );
}
