//! swarm-6k5: docs/rules.md is GENERATED from the `Rule` enum (name,
//! kind, meaning, what it detects, false-positive modes) — the catalog
//! cannot drift from the registry.
//!
//! Regenerate with: `BEVY_SWARM_UPDATE=schema cargo test --test docs_generation`

use bevy_swarm::rules::Rule;

fn generate_rules_md() -> String {
    let mut out = String::new();
    out.push_str("# Rule Catalog\n\n");
    out.push_str(
        "GENERATED from `src/rules.rs` (`Rule` enum) — do not edit by hand.\n\
         Regenerate with `BEVY_SWARM_UPDATE=schema cargo test --test docs_generation`.\n\
         Source of truth: `bevy_swarm::rules::Rule`.\n\
         Expectation files (X2) reference only these names.\n\n",
    );

    // Index table (kept compatible with the rules.rs meta-tests: a
    // `| `name` | kind |` row per rule).
    out.push_str("## Index\n\n| Rule name | Kind |\n|---|---|\n");
    for &rule in Rule::ALL {
        let kind = if rule.is_informational() {
            "informational"
        } else {
            "major"
        };
        out.push_str(&format!("| `{}` | {} |\n", rule.name(), kind));
    }

    out.push_str("\n## Reference\n\n");
    for &rule in Rule::ALL {
        out.push_str(&format!("### `{}`\n\n", rule.name()));
        out.push_str(&format!(
            "**Kind:** {} — {}\n\n",
            if rule.is_informational() {
                "informational (reported, never gates run status)"
            } else {
                "major (fails the run)"
            },
            rule.meaning()
        ));
        out.push_str(&format!("**Detects:** {}\n\n", rule.detects()));
        out.push_str(&format!(
            "**False-positive modes:** {}\n\n",
            rule.false_positive_modes()
        ));
    }

    out.push_str("Notes:\n");
    out.push_str("- `crash` is a report **status**, not a rule.\n");
    out.push_str(
        "- Informational rules are excluded from digests, failure signatures,\n\
         fingerprint gating and sweep pass/fail (see docs/conventions.md §8).\n",
    );
    out
}

/// Drift guard: docs/rules.md equals generated output.
#[test]
fn rules_doc_is_generated() {
    let generated = generate_rules_md();
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/rules.md");
    let committed = std::fs::read_to_string(&path).ok();

    if committed.as_deref() == Some(generated.as_str()) {
        return;
    }

    if std::env::var("BEVY_SWARM_UPDATE").is_ok() {
        std::fs::write(&path, generated).expect("write docs/rules.md");
        panic!("docs/rules.md REGENERATED — commit the diff");
    }
    panic!(
        "docs/rules.md is stale: the Rule registry changed without \
         regenerating. Run: BEVY_SWARM_UPDATE=schema cargo test --test docs_generation"
    );
}
