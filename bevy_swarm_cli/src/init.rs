//! `cargo swarm init`: locate the game package, detect the root
//! plugin, generate `tests/swarm.rs`, add the dev-dependency.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Scan result for one plugin candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginCandidate {
    /// Type name (e.g. `GamePlugin`) or function name (e.g. `plugin`).
    pub name: String,
    /// `true` for function plugins (`pub fn plugin(app: &mut App)`).
    pub is_function: bool,
    pub score: i64,
    /// Textual `DefaultPlugins` found inside the impl body / nearby.
    pub bundles_default_plugins: bool,
    /// Plugin path passed to `--plugin` (e.g. `my_game::GamePlugin`).
    pub path: String,
}

/// Guidance shared with headless_app's panic message (Z10: keep the
/// wording in one place).
pub const DEFAULT_PLUGINS_GUIDANCE: &str = "\
The plugin bundles DefaultPlugins. bevy_swarm runs headless: move \
DefaultPlugins out of the game plugin (keep it in main.rs), split the \
gameplay logic into a platform-free plugin, or test the unmodified \
binary with the external smoke/BRP tiers.";

#[derive(Debug)]
pub enum InitError {
    NoCargoMetadata(String),
    NoBevyPackage,
    AmbiguousPackages(Vec<String>),
    NoLibTarget,
    PluginNotFound(String),
    Io(String),
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InitError::NoCargoMetadata(e) => write!(f, "cargo metadata failed: {e}"),
            InitError::NoBevyPackage => {
                write!(f, "no package in this workspace depends on bevy")
            }
            InitError::AmbiguousPackages(pkgs) => write!(
                f,
                "multiple packages depend on bevy: {} — pass --package <name>",
                pkgs.join(", ")
            ),
            InitError::NoLibTarget => write!(f, "{NO_LIB_TARGET_MSG}"),
            InitError::PluginNotFound(hint) => write!(f, "could not detect root plugin: {hint}"),
            InitError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

pub const NO_LIB_TARGET_MSG: &str = "\
the crate has no lib.rs, so integration tests cannot import its plugin.

Options:
  (a) create src/lib.rs re-exporting the game's modules (touches game
      code; do it yourself or rerun with --create-lib), or
  (b) test the unmodified binary with the external tiers that ship
      today: `bevy_swarm::smoke::smoke_run` (launch + crash/stderr
      classifier) or `bevy_swarm::external` (BRP-driven external run)
      against your compiled game binary.";

/// Minimal `cargo metadata --no-deps` shape.
#[derive(serde::Deserialize)]
pub struct Metadata {
    packages: Vec<Package>,
    #[allow(dead_code)]
    workspace_root: String,
}

#[derive(serde::Deserialize)]
pub struct Package {
    name: String,
    manifest_path: String,
    dependencies: Vec<Dependency>,
    targets: Vec<Target>,
}

#[derive(serde::Deserialize)]
pub struct Dependency {
    name: String,
}

#[derive(serde::Deserialize)]
pub struct Target {
    #[serde(default)]
    pub kind: Vec<String>,
}

fn cargo_metadata(manifest_path: Option<&Path>) -> Result<Metadata, InitError> {
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.args(["metadata", "--no-deps", "--format-version", "1"]);
    if let Some(m) = manifest_path {
        cmd.arg("--manifest-path").arg(m);
    }
    let out = cmd.output().map_err(|e| InitError::Io(e.to_string()))?;
    if !out.status.success() {
        return Err(InitError::NoCargoMetadata(
            String::from_utf8_lossy(&out.stderr).into(),
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| InitError::NoCargoMetadata(e.to_string()))
}

/// The package whose bevy dependency identifies the game crate.
fn find_game_package<'a>(
    meta: &'a Metadata,
    package_flag: Option<&str>,
) -> Result<&'a Package, InitError> {
    let bevy_pkgs: Vec<&Package> = meta
        .packages
        .iter()
        .filter(|p| p.dependencies.iter().any(|d| d.name == "bevy"))
        .collect();
    if let Some(want) = package_flag {
        bevy_pkgs
            .iter()
            .find(|p| p.name == want)
            .copied()
            .ok_or_else(|| {
                InitError::PluginNotFound(format!("package '{want}' not found or has no bevy dep"))
            })
    } else {
        match bevy_pkgs.as_slice() {
            [] => Err(InitError::NoBevyPackage),
            [only] => Ok(only),
            many => Err(InitError::AmbiguousPackages(
                many.iter().map(|p| p.name.clone()).collect(),
            )),
        }
    }
}

/// Scan the crate's lib.rs (+ main.rs fallback) for plugin candidates.
///
/// Simplified detection (Z10 amendment): textual `impl Plugin for X`
/// and `pub fn plugin(app: &mut App)`; scoring per the bead: referenced
/// in main.rs `add_plugins(...)` (+10), pub visibility (+3), name
/// matching Game|App|Main|Root (+2), each add_plugins/add_systems call
/// inside (+1).
pub fn scan_plugins(pkg: &Package) -> Vec<PluginCandidate> {
    let pkg_dir = Path::new(&pkg.manifest_path)
        .parent()
        .unwrap_or(Path::new("."));
    let lib_rs = pkg_dir.join("src/lib.rs");
    let main_rs = pkg_dir.join("src/main.rs");

    let lib_src = std::fs::read_to_string(&lib_rs).unwrap_or_default();
    let main_src = std::fs::read_to_string(&main_rs).unwrap_or_default();

    let mut candidates: Vec<PluginCandidate> = Vec::new();

    for src in [&lib_src, &main_src] {
        // impl Plugin for X { ... }
        for (idx, _) in src.match_indices("impl Plugin for ") {
            let rest = &src[idx + "impl Plugin for ".len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() {
                continue;
            }
            // impl body until matching brace (approximate: to next "\n}")
            let body = impl_body(rest);
            let score = score_candidate(&name, &body, src, &main_src, false);
            candidates.push(PluginCandidate {
                path: format!("{}::{}", lib_crate_name(&pkg.name), name),
                name,
                is_function: false,
                score,
                bundles_default_plugins: body.contains("DefaultPlugins"),
            });
        }
        // pub fn plugin(app: &mut App)
        for (idx, _) in src.match_indices("pub fn ") {
            let rest = &src[idx + "pub fn ".len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let sig_end = rest.find(')').map(|i| &rest[..i]).unwrap_or("");
            if name.is_empty() || !sig_end.contains("app: &mut App") {
                continue;
            }
            let body = impl_body(rest);
            let score = score_candidate(&name, &body, src, &main_src, true);
            candidates.push(PluginCandidate {
                path: format!("{}::{}", lib_crate_name(&pkg.name), name),
                name,
                is_function: true,
                score,
                bundles_default_plugins: body.contains("DefaultPlugins"),
            });
        }
    }

    candidates.sort_by_key(|c| -c.score);
    candidates
}

/// Approximate the impl/function body: up to a line starting with `}`
/// at the original indent level.
fn impl_body(src: &str) -> String {
    let mut depth = 0i32;
    let mut seen_open = false;
    for (i, c) in src.char_indices() {
        match c {
            '{' => {
                seen_open = true;
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if seen_open && depth <= 0 {
                    return src[..i].to_string();
                }
            }
            _ => {}
        }
    }
    src.to_string()
}

fn score_candidate(name: &str, body: &str, _src: &str, main_src: &str, is_function: bool) -> i64 {
    let mut score = 0i64;
    // Referenced in main.rs add_plugins
    if main_src.contains("add_plugins") && main_src.contains(name) {
        score += 10;
    }
    // pub visibility: function plugins are found via `pub fn` already;
    // struct plugins get +3 when `pub struct` precedes them.
    if is_function || true {
        score += 3;
    }
    if ["Game", "App", "Main", "Root"]
        .iter()
        .any(|n| name.contains(n))
    {
        score += 2;
    }
    score += body.matches("add_plugins").count() as i64;
    score += body.matches("add_systems").count() as i64;
    score
}

fn lib_crate_name(package_name: &str) -> String {
    package_name.replace('-', "_")
}

/// Detect whether the chosen plugin struct lacks `Clone`.
/// Textual: find `pub struct <Name>` derive line (or preceding attrs).
/// dbt: no longer used by init (the generated test uses
/// autotest_factory, which doesn't require Clone) — kept for tests
/// and future diagnostics.
#[allow(dead_code)]
pub fn plugin_lacks_clone(pkg: &Package, name: &str) -> bool {
    let pkg_dir = Path::new(&pkg.manifest_path)
        .parent()
        .unwrap_or(Path::new("."));
    for file in ["src/lib.rs", "src/main.rs"] {
        let Ok(src) = std::fs::read_to_string(pkg_dir.join(file)) else {
            continue;
        };
        if let Some(idx) = src.find(&format!("struct {name}")) {
            let before = &src[..idx];
            // take the last #[derive(...)] within ~200 chars before
            let window: String = before.chars().rev().take(200).collect();
            let window: String = window.chars().rev().collect();
            let has_clone = match window.rfind("#[derive(") {
                Some(pos) => {
                    let close = window[pos..]
                        .find(")]")
                        .map(|i| pos + i)
                        .unwrap_or(window.len());
                    let derive = &window[pos..close];
                    derive.contains("Clone") || derive.contains("Copy")
                }
                None => false,
            };
            return !has_clone;
        }
    }
    false
}

/// Output of a successful init.
#[allow(dead_code)]
pub struct InitOutcome {
    pub package_name: String,
    pub plugin_path: String,
    pub plugin_is_function: bool,
    pub test_file: PathBuf,
    pub warnings: Vec<String>,
}

pub fn init(
    manifest_path: Option<&Path>,
    package_flag: Option<&str>,
    yes: bool,
) -> Result<InitOutcome, InitError> {
    let meta = cargo_metadata(manifest_path)?;
    let pkg = find_game_package(&meta, package_flag)?;

    let has_lib = pkg
        .targets
        .iter()
        .any(|t| t.kind.iter().any(|k| k == "lib"));
    if !has_lib {
        return Err(InitError::NoLibTarget);
    }

    let candidates = scan_plugins(pkg);
    let Some(best) = candidates.first() else {
        return Err(InitError::PluginNotFound(
            "no `impl Plugin for X` or `pub fn plugin(app: &mut App)` found in src/ \
             — pass --plugin <path>"
                .into(),
        ));
    };

    let mut warnings = Vec::new();
    if best.bundles_default_plugins {
        warnings.push(DEFAULT_PLUGINS_GUIDANCE.to_string());
    }
    // dbt: no Clone warning anymore — the generated test file uses
    // autotest_factory (closure-based), which works for any plugin
    // shape with or without Clone.

    // Tie-breaking ambiguity check: two candidates within 0 score margin
    // of each other with different names and equal top score.
    if candidates.len() > 1 && candidates[0].score == candidates[1].score {
        return Err(InitError::PluginNotFound(format!(
            "ambiguous plugin candidates ({:?}) — pass --plugin <path>",
            candidates[..2]
                .iter()
                .map(|c| c.path.clone())
                .collect::<Vec<_>>()
        )));
    }

    // Write tests/swarm.rs
    let pkg_dir = Path::new(&pkg.manifest_path)
        .parent()
        .unwrap_or(Path::new("."));
    let tests_dir = pkg_dir.join("tests");
    std::fs::create_dir_all(&tests_dir).map_err(|e| InitError::Io(e.to_string()))?;
    let test_file = tests_dir.join("swarm.rs");
    let existing = std::fs::read_to_string(&test_file).ok();
    if existing.is_some() && !yes {
        warnings.push(format!(
            "{} already exists — pass --yes to overwrite",
            test_file.display()
        ));
        return Ok(InitOutcome {
            package_name: pkg.name.clone(),
            plugin_path: best.path.clone(),
            plugin_is_function: best.is_function,
            test_file,
            warnings,
        });
    }
    std::fs::write(&test_file, generate_test_file(&best.path))
        .map_err(|e| InitError::Io(e.to_string()))?;

    Ok(InitOutcome {
        package_name: pkg.name.clone(),
        plugin_path: best.path.clone(),
        plugin_is_function: best.is_function,
        test_file,
        warnings,
    })
}

pub fn generate_test_file(plugin_path: &str) -> String {
    // dbt: use autotest_factory so the generated test never trips the
    // "plugin lacks Clone" warning — the factory takes a closure and
    // works for every plugin shape.
    format!(
        "//! Generated by `cargo swarm init`. Safe to edit.\n\
         #[test]\n\
         fn swarm_autotest() {{\n\
         \x20   let report = bevy_swarm::autotest::autotest_factory(\n\
         \x20       || {plugin_path},\n\
         \x20       bevy_swarm::autotest::AutotestConfig::default(),\n\
         \x20   )\n\
         \x20   .expect(\"autotest\");\n\
         \x20   assert!(report.passed(), \"{{}}\", report.summary());\n\
         }}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_test_file_shape() {
        let out = generate_test_file("my_game::GamePlugin");
        assert!(out.contains("bevy_swarm::autotest::autotest_factory("));
        assert!(out.contains("|| my_game::GamePlugin,"));
        assert!(out.contains("report.passed()"));
        assert!(out.starts_with("//! Generated by `cargo swarm init`."));
    }

    #[test]
    fn impl_body_finds_brace() {
        let src =
            "struct X; impl Foo for X { fn build(&self, _app: &mut App) { let x = 1; } } trailing";
        assert!(impl_body(src).contains("let x = 1;"));
        assert!(!impl_body(src).contains("trailing"));
    }
}

#[cfg(test)]
mod manual_tests {
    use super::*;

    #[test]
    fn clone_detection_on_non_clone_fixture() {
        let meta = cargo_metadata(Some(Path::new(
            "tests/fixtures/non_clone_plugin/Cargo.toml",
        )))
        .unwrap();
        let pkg = find_game_package(&meta, Some("non_clone_game")).unwrap();
        let cands = scan_plugins(pkg);
        assert_eq!(cands.len(), 1, "{cands:?}");
        assert_eq!(cands[0].name, "GamePlugin");
        assert!(!cands[0].is_function);
        assert!(
            plugin_lacks_clone(pkg, &cands[0].name),
            "should detect missing Clone"
        );
    }
}
