//! `cargo swarm report`: pretty-print the last autotest report
//! (conventions::runs_dir()/last/report.json).

use std::path::PathBuf;

pub fn last_report_path() -> PathBuf {
    // autotest writes runs/<dir>/report.json and a `last` POINTER file
    // containing the run-dir name.
    let runs = bevy_swarm::conventions::runs_dir();
    let pointer = runs.join("last");
    match std::fs::read_to_string(&pointer) {
        Ok(dir_name) => runs.join(dir_name.trim()).join("report.json"),
        Err(_) => runs.join("last").join("report.json"),
    }
}

pub fn report() -> Result<i32, String> {
    let path = last_report_path();
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("no report found at {}: {e}", path.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("report is not valid JSON: {e}"))?;

    let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("-");
    let tier = v
        .get("contract_tier")
        .and_then(|s| s.as_str())
        .unwrap_or("-");
    let mode = v
        .get("harness_mode")
        .and_then(|s| s.as_str())
        .unwrap_or("-");
    println!("harness: {mode}  tier: {tier}  status: {status}");

    if let Some(scope) = v.get("scope") {
        println!(
            "seeds: {}  duration: {}s",
            scope.get("seeds_run").and_then(|x| x.as_u64()).unwrap_or(0),
            scope
                .get("total_duration_s")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0)
        );
        // bbb: inactive scope families carry genuine signal (why an
        // oracle family could NOT run) — never drop them silently.
        if let Some(inactive) = scope.get("inactive").and_then(|i| i.as_array()) {
            if !inactive.is_empty() {
                println!("inactive checks ({}):", inactive.len());
                for entry in inactive {
                    // Serialized as [family, reason] pairs (tuples).
                    let parts: Vec<&str> = entry
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                        .unwrap_or_default();
                    let (family, reason) = match parts.as_slice() {
                        [f, r] => (*f, *r),
                        [f] => (*f, ""),
                        _ => ("?", "?"),
                    };
                    println!("  [{family}] {reason}");
                }
            }
        }
    }
    // bbb: warnings (Z19 stderr classifier, informational notes) —
    // surfaced alongside failures, never silently dropped.
    if let Some(warns) = v.get("warnings").and_then(|w| w.as_array()) {
        if !warns.is_empty() {
            println!("warnings ({}):", warns.len());
            for w in warns {
                println!("  - {}", w.as_str().unwrap_or("?"));
            }
        }
    }
    if let Some(fails) = v.get("failures").and_then(|f| f.as_array()) {
        if fails.is_empty() {
            println!("failures: none");
        } else {
            println!("failures ({}):", fails.len());
            for f in fails {
                println!(
                    "  [{}] {} (x{})",
                    f.get("rule").and_then(|r| r.as_str()).unwrap_or("?"),
                    f.get("fingerprint")
                        .and_then(|x| x.get("0"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("?"),
                    f.get("occurrences").and_then(|x| x.as_u64()).unwrap_or(0)
                );
            }
        }
    }
    if let Some(runs) = v.get("runs").and_then(|r| r.as_array()) {
        println!("runs:");
        for r in runs {
            println!(
                "  seed {}: {} ({} violations, {} frames, {}ms)",
                r.get("seed").and_then(|x| x.as_u64()).unwrap_or(0),
                r.get("status").and_then(|x| x.as_str()).unwrap_or("?"),
                r.get("violations_count")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0),
                r.get("frame_count").and_then(|x| x.as_u64()).unwrap_or(0),
                r.get("duration_ms").and_then(|x| x.as_u64()).unwrap_or(0)
            );
        }
    }
    Ok(0)
}
