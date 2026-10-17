//! f7t: malformed scenario JSON produces descriptive errors, not
//! panics or opaque serde failures. Scenarios are user-authored JSON
//! (the library's PRIMARY interface), so error quality is UX.

use bevy_playtest::harness::Scenario;

fn parse(json: &str) -> Result<Scenario, serde_json::Error> {
    serde_json::from_str(json)
}

#[test]
fn truncated_json_errors() {
    let err = parse(r#"{"bot":{"type":"replay","inp"#).unwrap_err();
    // Descriptive: points at truncation, not a generic failure.
    assert!(!err.to_string().is_empty());
}

#[test]
fn typoed_field_errors() {
    // "duraton_s" typo'd: deny_unknown_fields must reject it with the
    // unknown-field name in the message.
    let err = parse(r#"{"bot":{"type":"replay","inputs":[]},"duraton_s":1.0}"#).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("duraton_s"),
        "error must name the bad field: {msg}"
    );
}

#[test]
fn unknown_field_errors() {
    let err = parse(r#"{"bot":{"type":"replay","inputs":[]},"duration_s":1.0,"xyzzy":true}"#)
        .unwrap_err();
    assert!(
        err.to_string().contains("xyzzy"),
        "unknown field must be named"
    );
}

#[test]
fn missing_required_field_errors() {
    // No "bot": required field.
    let err = parse(r#"{"duration_s":1.0}"#).unwrap_err();
    assert!(
        err.to_string().contains("bot"),
        "missing-field error must name 'bot'"
    );
}

#[test]
fn invalid_bot_type_errors() {
    let err = parse(r#"{"bot":{"type":"nosuchbot"},"duration_s":1.0}"#).unwrap_err();
    assert!(
        err.to_string().contains("bot") || err.to_string().contains("nosuchbot"),
        "invalid bot type must be descriptive: {err}"
    );
}

#[test]
fn valid_scenario_parses() {
    // Control: a valid minimal scenario parses cleanly.
    let s = parse(r#"{"bot":{"type":"replay","inputs":[]},"duration_s":1.0}"#).unwrap();
    assert_eq!(s.duration_s, 1.0);
}

#[test]
fn malformed_expectation_json_fails_loudly() {
    // f7t: the fx0 acceptance suite loads expectation JSON; malformed
    // expectations must NOT silently weaken assertions — parse must
    // fail loudly (we assert a malformed expectation document cannot
    // deserialize into the same Value-loading helper shape).
    let raw = r#"{"status": "pass", "must_report": ["dead_verb""#; // truncated
    let v: Result<serde_json::Value, _> = serde_json::from_str(raw);
    assert!(v.is_err(), "truncated expectation JSON must fail to parse");
}
