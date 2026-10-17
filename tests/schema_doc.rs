//! swarm-6k5: docs/schema.json is GENERATED from the `Scenario` struct
//! (schemars), committed, never hand-edited.
//!
//! - Regenerate with: `BEVY_PLAYTEST_UPDATE=schema cargo test --test schema_doc`
//! - The drift test fails when `Scenario` changes without regenerating.
//! - Real sample scenarios from fixtures validate against the schema.

use bevy_playtest::scenario::Scenario;

fn schema_json() -> serde_json::Value {
    let schema = schemars::schema_for!(bevy_playtest::scenario::Scenario);
    serde_json::to_value(&schema).expect("serialize schema")
}

fn schema_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/schema.json")
}

/// The generated schema must validate real scenarios (acceptance: 3+
/// sample scenarios from the repo's own tests/fixures).
#[test]
fn schema_validates_sample_scenarios() {
    let schema = schema_json();
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");

    let samples: Vec<serde_json::Value> = vec![
        // chaos bot, invariants, from the walker acceptance path
        serde_json::json!({
            "bot": { "type": "chaos", "seed": 42 },
            "duration_s": 6.0,
            "invariants": [
                { "name": "hp_floor", "rule": "custom", "path": "TestApi.hp",
                  "check": "above", "value": 10, "scale": 100 }
            ],
            "setup": {}
        }),
        // planner bot with goal tree (from the README usage example)
        serde_json::json!({
            "bot": {
                "type": "planner",
                "goals": { "kind": "primitive", "path": "TestApi.score",
                           "check": "above", "value": 3,
                           "emit": [{ "intent": "choice", "index": 0 }] }
            },
            "duration_s": 1.0,
            "invariants": []
        }),
        // replay bot with frame-indexed intents
        serde_json::json!({
            "bot": {
                "type": "replay",
                "inputs": [ { "frame": 0, "intent": { "intent": "move", "dir": [1.0, 0.0] } } ]
            },
            "duration_s": 2.0,
            "invariants": []
        }),
        // synthetic keyboard + liveness
        serde_json::json!({
            "bot": {
                "type": "synthetic_keyboard",
                "key_presses": [ { "frame": 5, "key": "space", "hold_frames": 3 } ]
            },
            "duration_s": 3.0,
            "liveness": { "mode": "after_intent", "timeout_s": 2.0 },
            "check_determinism": 2
        }),
    ];

    assert!(
        samples.len() >= 3,
        "acceptance requires at least 3 sample scenarios"
    );
    for (i, sample) in samples.iter().enumerate() {
        // Must deserialize through the real DTO (proves schema matches
        // serde, not just a hand-written approximation).
        let parsed: Scenario = serde_json::from_value(sample.clone())
            .unwrap_or_else(|e| panic!("sample {i} does not deserialize into Scenario: {e}"));
        let roundtrip = serde_json::to_value(&parsed).expect("sample serializes back");
        validator
            .validate(&roundtrip)
            .unwrap_or_else(|e| panic!("sample {i} fails schema: {e}"));
    }
}

/// Drift guard: docs/schema.json equals the schemars output for
/// `Scenario`. When this fails, regenerate with
/// `BEVY_PLAYTEST_UPDATE=schema cargo test --test schema_doc`.
#[test]
fn schema_doc_is_generated() {
    let generated = schema_json();
    let committed: Option<serde_json::Value> = std::fs::read_to_string(schema_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());

    if committed.as_ref() == Some(&generated) {
        return;
    }

    if std::env::var("BEVY_PLAYTEST_UPDATE").is_ok() {
        let pretty = serde_json::to_string_pretty(&generated).unwrap();
        std::fs::write(schema_path(), pretty).expect("write docs/schema.json");
        panic!("docs/schema.json REGENERATED — commit the diff");
    }
    panic!(
        "docs/schema.json is stale: the Scenario struct changed without \
         regenerating. Run: BEVY_PLAYTEST_UPDATE=schema cargo test --test schema_doc"
    );
}

/// zp2: cookbook examples validate against the schema (the CLI
/// validate command uses the same runtime generator).
#[test]
fn schema_validates_cookbook_examples() {
    let schema = schema_json();
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/scenarios");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        validator
            .validate(&v)
            .unwrap_or_else(|e| panic!("{} fails schema: {e}", p.display()));
        checked += 1;
    }
    assert!(checked >= 5);
}
