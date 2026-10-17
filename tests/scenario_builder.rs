//! zp2: examples/scenarios cookbook — every shipped example parses,
//! deserializes into Scenario, and passes load-time validation.
//! Also: ScenarioBuilder round-trip parity with hand-written JSON.

use bevy_playtest::builder::ScenarioBuilder;
use bevy_playtest::scenario::Scenario;

fn examples_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/scenarios")
}

#[test]
fn cookbook_examples_parse_and_validate() {
    let mut count = 0;
    for entry in std::fs::read_dir(examples_dir()).expect("examples/scenarios exists") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).expect("readable");
        let scenario: Scenario = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("{}: deserialize failed: {e}", path.display()));
        bevy_playtest::scenario::validate_scenario(&scenario)
            .unwrap_or_else(|e| panic!("{}: validation failed: {e}", path.display()));
        count += 1;
    }
    assert!(
        count >= 5,
        "expected at least 5 cookbook examples, saw {count}"
    );
}

/// zp2 acceptance: builder output is JSON-identical to a hand-written
/// example (menu-navigation.json).
#[test]
fn builder_matches_handwritten_json() {
    let raw = std::fs::read_to_string(examples_dir().join("menu-navigation.json")).unwrap();
    let handwritten: serde_json::Value = serde_json::from_str(&raw).unwrap();

    let built = ScenarioBuilder::pointer()
        .click_actionable("ButtonSettings", 10)
        .click_actionable("ButtonAudio", 90)
        .click_hold("ButtonSliderMaster", 170, 30)
        .configure(|b| {
            b.name("menu-navigation").duration_s(12.0).eventually(
                "reaches-options-menu",
                "TestApi.menu_phase",
                "equals",
                "options",
                8.0,
            )
        })
        .build();
    let built_json: serde_json::Value = serde_json::to_value(&built).unwrap();

    // Compare normalized (deserialized-then-reserialized) forms.
    let hw: Scenario = serde_json::from_value(handwritten).unwrap();
    assert_eq!(
        serde_json::to_value(&hw).unwrap(),
        built_json,
        "builder output must equal hand-written example JSON"
    );
}

/// zp2: keyboard builder parity with a hand-written equivalent.
#[test]
fn keyboard_builder_matches_expected_json() {
    let built = ScenarioBuilder::keyboard()
        .press("Space", 15)
        .press("Escape", 90)
        .hold("w", 380, 45)
        .configure(|b| b.name("kb").duration_s(10.0).tps(60))
        .build();

    let handwritten: Scenario = serde_json::from_str(
        r#"{"name":"kb","bot":{"type":"synthetic_keyboard","key_presses":[
            {"frame":15,"key":"Space"},
            {"frame":90,"key":"Escape"},
            {"frame":380,"key":"w","hold_frames":45}]},"tps":60,"duration_s":10.0}"#,
    )
    .unwrap();

    assert_eq!(
        serde_json::to_value(&built).unwrap(),
        serde_json::to_value(&handwritten).unwrap()
    );
}

/// zp2: builder rejects invalid configuration loudly at author time
/// (try_build) instead of at run time.
#[test]
fn builder_rejects_invalid_scenarios() {
    let err = ScenarioBuilder::chaos()
        .duration_s(0.0)
        .try_build()
        .expect_err("duration 0 must be rejected");
    let msg = err.to_string();
    assert!(msg.contains("duration_s"), "{msg}");
}

/// zp2: unknown check names panic with guidance (documented behavior
/// of the builder — the compile-time-ish typo feedback JSON lacks).
#[test]
#[should_panic(expected = "unknown check 'geq'")]
fn builder_panics_on_unknown_check() {
    let _ = ScenarioBuilder::chaos()
        .custom_invariant("x", "TestApi.hp", "geq", 1)
        .build();
}
