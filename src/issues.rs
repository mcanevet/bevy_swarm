//! T13 (swarm-qa8.13): persistent issue DB + new/regressed gate.
//!
//! The DB lives at `tests/swarm/issues.json` (conventions §U1):
//! records keyed by T1 fingerprint with state open|muted|resolved.
//! Writes happen ONLY under `BEVY_PLAYTEST_UPDATE=issues` (CI reads).
//! The classification + gate are library functions so Z18's autotest
//! can adopt the default gate (NewOrRegressed).

use crate::conventions;
use crate::fingerprint::Fingerprint;
use std::collections::HashMap;

/// State of a tracked issue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueState {
    Open,
    Muted,
    Resolved,
}

/// One known-issue record in the DB.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct IssueRecord {
    pub state: IssueState,
    /// Short human title.
    pub title: String,
    /// First/last seen ISO timestamps.
    pub first_seen: String,
    pub last_seen: String,
    /// Occurrence count across all runs.
    pub occurrences: u64,
    /// Seeds the issue was seen on.
    pub seeds: Vec<u64>,
    /// Rule name (denormalized for convenience).
    pub rule: String,
}

impl Default for IssueRecord {
    fn default() -> Self {
        Self {
            state: IssueState::Open,
            title: String::new(),
            first_seen: String::new(),
            last_seen: String::new(),
            occurrences: 0,
            seeds: vec![],
            rule: String::new(),
        }
    }
}

/// The issue DB: fingerprint -> record.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct IssueDb {
    pub schema_version: u32,
    pub issues: HashMap<Fingerprint, IssueRecord>,
}

impl IssueDb {
    /// Load from the conventions path. Missing file = empty DB.
    pub fn load() -> Self {
        Self::load_from(&conventions::issues_path())
    }

    /// Load from an explicit path. A missing file yields an empty DB
    /// (the common case in a fresh checkout).
    pub fn load_from(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!(
                    "WARNING: malformed issue DB {} ({}); treating as empty",
                    path.display(),
                    e
                );
                Self::default()
            }),
            Err(_) => Self {
                schema_version: conventions::SCHEMA_VERSION,
                issues: HashMap::new(),
            },
        }
    }

    /// Persist to the conventions path (only under update mode).
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&conventions::issues_path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
    }

    /// Classify observed fingerprints against the DB.
    pub fn classify(&self, observed: &[(Fingerprint, u64)]) -> GateSummary {
        let mut summary = GateSummary::default();
        for (fp, seed) in observed {
            match self.issues.get(fp).map(|r| r.state) {
                None => summary.new.push((*fp).clone()),
                Some(IssueState::Open) => summary.known.push((*fp).clone()),
                Some(IssueState::Muted) => summary.muted.push((*fp).clone()),
                // Seen again after being resolved: REGRESSED.
                Some(IssueState::Resolved) => summary.regressed.push((*fp).clone()),
            }
            let _ = seed;
        }
        summary
    }

    /// Record observed occurrences (first_seen/last_seen/occurrences/
    /// seeds). Returns a mutated copy; persistence is the caller's job.
    pub fn record(&mut self, observed: &[(Fingerprint, u64, String, u64)]) {
        for (fp, seed, rule, _count) in observed {
            let now = conventions::iso_timestamp(std::time::SystemTime::now());
            let entry = self.issues.entry((*fp).clone()).or_default();
            if entry.title.is_empty() {
                entry.title = format!("{rule} {}", fp.0);
            }
            if entry.first_seen.is_empty() {
                entry.first_seen = now.clone();
            }
            entry.last_seen = now;
            entry.occurrences += 1;
            entry.rule = rule.clone();
            if !entry.seeds.contains(seed) {
                entry.seeds.push(*seed);
            }
        }
    }
}

/// Classification of observed failures: the gate input.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct GateSummary {
    pub new: Vec<Fingerprint>,
    pub regressed: Vec<Fingerprint>,
    pub known: Vec<Fingerprint>,
    pub muted: Vec<Fingerprint>,
}

impl GateSummary {
    /// Fingerprints the gate considers blocking.
    pub fn blocking(&self) -> Vec<Fingerprint> {
        let mut v: Vec<Fingerprint> = self
            .new
            .iter()
            .chain(self.regressed.iter())
            .cloned()
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v.dedup();
        v
    }
}

/// CI gate over the classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Default: fail only on new or regressed issues.
    NewOrRegressed,
    /// Fail on ANY open issue (including known-open).
    AnyOpen,
    /// No gating (report only).
    None,
}

impl Gate {
    /// Apply the gate to a classification summary.
    pub fn apply(self, summary: &GateSummary) -> bool {
        match self {
            Gate::NewOrRegressed => summary.blocking().is_empty(),
            Gate::AnyOpen => summary.blocking().is_empty() && summary.known.is_empty(),
            Gate::None => true,
        }
    }
}

/// Serde-friendly mirror of [`Gate`] for `AutotestConfig`
/// (scenario JSON carries the config; the enum itself stays plain).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateSerde {
    #[default]
    NewOrRegressed,
    AnyOpen,
    None,
}

impl From<GateSerde> for Gate {
    fn from(v: GateSerde) -> Gate {
        match v {
            GateSerde::NewOrRegressed => Gate::NewOrRegressed,
            GateSerde::AnyOpen => Gate::AnyOpen,
            GateSerde::None => Gate::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(n: &str) -> Fingerprint {
        Fingerprint(n.to_string())
    }

    #[test]
    fn gate_new_or_regressed_ignores_known_open() {
        let mut db = IssueDb::default();
        db.issues.insert(
            fp("known"),
            IssueRecord {
                state: IssueState::Open,
                ..Default::default()
            },
        );
        let summary = db.classify(&[(fp("known"), 1), (fp("fresh"), 2)]);
        assert_eq!(summary.known, vec![fp("known")]);
        assert_eq!(summary.new, vec![fp("fresh")]);
        assert!(!Gate::NewOrRegressed.apply(&summary));
        let only_known = db.classify(&[(fp("known"), 1)]);
        assert!(Gate::NewOrRegressed.apply(&only_known));
    }

    #[test]
    fn regressed_detected() {
        let mut db = IssueDb::default();
        db.issues.insert(
            fp("fixed"),
            IssueRecord {
                state: IssueState::Resolved,
                ..Default::default()
            },
        );
        let summary = db.classify(&[(fp("fixed"), 3)]);
        assert_eq!(summary.regressed, vec![fp("fixed")]);
        assert!(!Gate::NewOrRegressed.apply(&summary));
    }

    #[test]
    fn db_roundtrip_stable_ordering() {
        let dir = std::env::temp_dir().join(format!("t13-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("issues.json");

        let mut db = IssueDb::default();
        db.record(&[
            (fp("b"), 1, "panic".into(), 1),
            (fp("a"), 2, "dead_verb".into(), 1),
        ]);
        db.save_to(&path).unwrap();
        let loaded = IssueDb::load_from(&path);
        let mut keys: Vec<String> = loaded.issues.keys().map(|f| f.0.clone()).collect();
        keys.sort();
        assert_eq!(keys, vec!["a".to_string(), "b".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
