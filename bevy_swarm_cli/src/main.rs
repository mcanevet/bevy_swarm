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
