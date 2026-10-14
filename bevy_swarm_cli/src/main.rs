//! cargo-swarm: CLI scaffolding for bevy_swarm playtests.
//!
//! Subcommands:
//! - `cargo swarm init [--package <name>] [--yes]` — detect the game's
//!   root plugin and generate `tests/swarm.rs` (safe to edit).
//! - `cargo swarm run [--seeds N] [--duration S]` — run the generated
//!   test with env overrides; `--plugin <path>` uses the external
//!   runner (X1 CLI mode) that never touches the game's dev-deps.
//! - `cargo swarm report` — pretty-print the last autotest report.
//!
//! Install (until the user approves publishing):
//! `cargo install --path bevy_swarm_cli` from the bevy_swarm checkout.

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // cargo invokes binaries as `cargo-swarm swarm ...` — the first
    // arg is the subcommand after the binary name.
    let args: Vec<String> = if args.first().map(String::as_str) == Some("swarm") {
        args.into_iter().skip(1).collect()
    } else {
        args
    };

    let code = match args.first().map(String::as_str) {
        Some("init") => cmd_init(&args[1..]),
        Some("run") => cmd_run(&args[1..]),
        Some("validate") => cmd_validate(&args[1..]),
        Some("report") => cmd_report(),
        Some("--help") | Some("help") | Some("-h") | None => {
            print_usage();
            0
        }
        Some(other) => {
            eprintln!("unknown subcommand: {other}");
            print_usage();
            2
        }
    };
    std::process::exit(code);
}

fn print_usage() {
    println!(
        "cargo-swarm — bevy_swarm CLI\n\
         \n\
         USAGE:\n\
         \x20   cargo swarm <SUBCOMMAND>\n\
         \n\
         SUBCOMMANDS:\n\
         \x20   init    Detect the game's root plugin and generate tests/swarm.rs\n\
         \x20   run     Run the generated swarm test (env overrides)\n\
         \x20   validate <file>  Check a scenario JSON without running the game\n\
         \x20   report  Pretty-print the last autotest report\n\
         \n\
         OPTIONS (init):\n\
         \x20   --package <name>   Select the game package (ambiguous bevy deps)\n\
         \x20   --plugin <path>    Override plugin detection with an explicit path\n\
         \x20   --yes              Overwrite existing tests/swarm.rs\n\
         \n\
         OPTIONS (run):\n\
         \x20   --manifest-path <p>  Target manifest\n\
         \x20   --package <name>     Target package\n\
         \x20   --seeds <N>          Seed count override\n\
         \x20   --duration <S>       Per-seed duration override\n\
         \x20   --report-json <p>    Copy report to a path\n\
         \x20   --plugin <path>      External runner mode (X1)\n"
    );
}

fn cmd_init(args: &[String]) -> i32 {
    let mut package: Option<String> = None;
    let mut yes = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--package" => package = it.next().cloned(),
            "--yes" | "-y" => yes = true,
            other => {
                eprintln!("init: unknown flag {other}");
                return 2;
            }
        }
    }
    match init::init(None, package.as_deref(), yes) {
        Ok(outcome) => {
            println!(
                "generated {} (plugin: {}, package: {})",
                outcome.test_file.display(),
                outcome.plugin_path,
                outcome.package_name
            );
            println!("add bevy_swarm to [dev-dependencies] if not present");
            for w in &outcome.warnings {
                eprintln!("warning: {w}");
            }
            0
        }
        Err(e) => {
            eprintln!("init failed: {e}");
            1
        }
    }
}

fn cmd_run(args: &[String]) -> i32 {
    let mut opts = run::RunOptions {
        manifest_path: None,
        package: None,
        seeds: None,
        duration: None,
        report_json: None,
        plugin: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--manifest-path" => {
                opts.manifest_path = it.next().map(PathBuf::from);
            }
            "--package" => opts.package = it.next().cloned(),
            "--seeds" => {
                opts.seeds = it.next().and_then(|v| v.parse().ok());
            }
            "--duration" => {
                opts.duration = it.next().and_then(|v| v.parse().ok());
            }
            "--report-json" => opts.report_json = it.next().map(PathBuf::from),
            "--plugin" => opts.plugin = it.next().cloned(),
            other => {
                eprintln!("run: unknown flag {other}");
                return 2;
            }
        }
    }
    let result = if opts.plugin.is_some() {
        run::run_external(&opts)
    } else {
        run::run(&opts)
    };
    match result {
        Ok(code) => {
            if let Some(dest) = &opts.report_json {
                if let Err(e) = std::fs::copy(report::last_report_path(), dest) {
                    eprintln!("warning: could not copy report to {}: {e}", dest.display());
                }
            }
            code
        }
        Err(e) => {
            eprintln!("run failed: {e}");
            1
        }
    }
}

fn cmd_validate(args: &[String]) -> i32 {
    let Some(file) = args.first() else {
        eprintln!("validate: expected a scenario JSON path");
        return 2;
    };
    if args.len() > 1 {
        eprintln!("validate: unexpected extra argument '{}'", args[1]);
        return 2;
    }
    let raw = match std::fs::read_to_string(file) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("validate: cannot read {file}: {e}");
            return 1;
        }
    };

    let mut errors: Vec<String> = Vec::new();

    // 1. Syntax: parse the JSON document itself.
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("validate: {file} is not valid JSON: {e}");
            return 1;
        }
    };

    // 2. Schema conformance: unknown fields, wrong types. Collected
    // per error, not first-abort.
    let schema = bevy_swarm::scenario::scenario_json_schema();
    if let Ok(validator) = jsonschema::validator_for(&schema) {
        if let Err(first) = validator.validate(&value) {
            // validate() returns only the first error; iter_errors
            // walks them all. Cap output at 10.
            let mut schema_errors = 0;
            for err in validator.iter_errors(&value) {
                schema_errors += 1;
                if schema_errors <= 10 {
                    let at = err.instance_path.to_string();
                    let at = if at.is_empty() {
                        String::new()
                    } else {
                        format!(" at '{at}'")
                    };
                    errors.push(format!("schema: {err}{at}"));
                }
            }
            if schema_errors == 0 {
                errors.push(format!("schema: {first}"));
            }
            if schema_errors > 10 {
                errors.push(format!(
                    "schema: ... and {} more errors",
                    schema_errors - 10
                ));
            }
        }
    }

    // 3. Semantic checks (validate_scenario): known where possible,
    // all reported at once.
    match serde_json::from_value::<bevy_swarm::scenario::Scenario>(value) {
        Ok(scenario) => {
            if let Err(e) = bevy_swarm::scenario::validate_scenario(&scenario) {
                errors.push(
                    e.to_string()
                        .trim_start_matches("scenario rejected: ")
                        .to_string(),
                );
            }
        }
        Err(e) => {
            // serde errors are usually covered by schema errors above;
            // keep them anyway when schema validation was silent.
            if errors.is_empty() {
                errors.push(format!("deserialize: {e}"));
            }
        }
    }

    if errors.is_empty() {
        println!("{file}: OK");
        0
    } else {
        eprintln!("{file}: {} error(s):", errors.len());
        for e in &errors {
            eprintln!("  - {e}");
        }
        1
    }
}

fn cmd_report() -> i32 {
    match report::report() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("report failed: {e}");
            1
        }
    }
}

mod init;
mod report;
mod run;
