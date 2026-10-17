//! Detect the resolved `bevy` minor version from Cargo.lock and expose
//! it as `bevy_0_19` / `bevy_0_20` cfgs so a single source tree can
//! support both. Cargo.lock is authoritative for what will actually be
//! compiled against.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");

    // Locate Cargo.lock by walking up from the manifest dir.
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let lock = locate_lock(&manifest_dir).expect("Cargo.lock not found above manifest dir");

    let minor = parse_bevy_minor(&lock).expect("bevy package entry not found in Cargo.lock");
    let cfg = match minor {
        19 => "bevy_0_19",
        20 => "bevy_0_20",
        other => panic!("unsupported bevy 0.{other} (expected 0.19 or 0.20)"),
    };

    // Modern cargo uses the `cargo::` spelling; older cargo only accepts
    // `cargo:`. Emit both spellings — unknown directives of the opposite
    // spelling are ignored with a warning at worst.
    println!("cargo:rustc-cfg={cfg}");
    println!("cargo::rustc-cfg={cfg}");
    println!("cargo:rustc-check-cfg=cfg(bevy_0_19,bevy_0_20)");
    println!("cargo::rustc-env=BEVY_MINOR={minor}");
    println!("cargo::rustc-check-cfg=cfg(bevy_0_19)");
    println!("cargo::rustc-check-cfg=cfg(bevy_0_20)");
}

fn locate_lock(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join("Cargo.lock");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Find the `[[package]] name = "bevy"` entry and return its minor version.
fn parse_bevy_minor(lock: &Path) -> Option<u64> {
    let text = fs::read_to_string(lock).ok()?;
    let mut in_bevy = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("name = ") {
            in_bevy = line == "name = \"bevy\"";
        } else if in_bevy && line.starts_with("version = ") {
            let v = line.trim_start_matches("version = ").trim_matches('"');
            let minor = v.split('.').nth(1)?;
            return minor.parse().ok();
        }
    }
    None
}
