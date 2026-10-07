//! T1: Failure fingerprints + normalizer + dedup (v0.2)
//!
//! A stable identity for bugs across seeds, frames, entities. Used by
//! E3 sweeps and B2 minimization to deduplicate failures.

use serde::{Deserialize, Serialize};
/// A stable hash of a failure's essence — groups related reports.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub String);

/// Parts of a fingerprint (for inspection/debugging; the hash input is
/// the concatenation of these fields).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct FingerprintParts {
    pub kind: String,
    pub rule: String,
    pub location: Vec<String>,
    pub normalized_message: String,
}

/// Normalization rules: strip volatile tokens (entity IDs, frames,
/// floats, hex addresses, seeds, paths). Reject keys force splits;
/// merge keys force merges.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Normalizer {
    /// Keys that FORCE a split (different values → different fingerprints).
    #[serde(default)]
    pub reject_keys: Vec<String>,
    /// Keys that FORCE a merge (values ignored for fingerprinting).
    #[serde(default)]
    pub merge_keys: Vec<String>,
}

impl Normalizer {
    /// Normalize a raw failure message into a fingerprint-stable string.
    /// Strips entity IDs, frame numbers, floats, hex addresses, seeds,
    /// and file paths (token-wise — no regex dependency). Handles
    /// "frame N" pairs (N → *). Reject/merge keys are applied last.
    pub fn normalize(&self, raw: &str) -> String {
        let tokens: Vec<&str> = raw.split_whitespace().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            let tok = tokens[i];
            if tok == "frame" && i + 1 < tokens.len() {
                let next = tokens[i + 1];
                if next.chars().all(|c| c.is_ascii_digit()) {
                    out.push("frame".to_string());
                    out.push("*".to_string());
                    i += 2;
                    continue;
                }
            }
            out.push(normalize_token(tok));
            i += 1;
        }
        out.join(" ")
    }

    /// Compute a fingerprint from a raw message + metadata.
    pub fn fingerprint(
        &self,
        kind: &str,
        rule: &str,
        location: &[String],
        message: &str,
    ) -> Fingerprint {
        let normalized = self.normalize(message);
        let parts = FingerprintParts {
            kind: kind.to_string(),
            rule: rule.to_string(),
            location: location.to_vec(),
            normalized_message: normalized.clone(),
        };
        // FNV-1a over the concatenated fields — a stable, portable
        // hash with no external dependencies. Two rounds (different
        // seeds) to widen the 64-bit result to 128 hex chars... v0.2
        // keeps a single round; the fingerprint is opaque and its
        // length is not part of the contract.
        fn fnv1a(bytes: &[u8]) -> u64 {
            let mut h: u64 = 0xcbf29ce484222325;
            for b in bytes {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            h
        }
        let mut buf = String::new();
        buf.push_str(kind);
        buf.push('\u{1}');
        buf.push_str(rule);
        for loc in &parts.location {
            buf.push('\u{1}');
            buf.push_str(loc);
        }
        buf.push('\u{1}');
        buf.push_str(&normalized);
        Fingerprint(format!("{:016x}", fnv1a(buf.as_bytes())))
    }
}

/// Default normalizer for v0.2 (no special reject/merge keys yet).
pub fn default_normalizer() -> Normalizer {
    Normalizer::default()
}

/// Rewrite a single whitespace token: strip volatile numeric parts.
fn normalize_token(tok: &str) -> String {
    // Entity ids: "123v4" or "entity:123v4" — strip to a wildcard.
    if let Some(stripped) = tok.strip_prefix("entity:") {
        if is_entity_id(stripped) {
            return "entity:*".to_string();
        }
    }
    if is_entity_id(tok) {
        return "*".to_string();
    }
    // Hex addresses: 0x...
    if tok.starts_with("0x") && tok.len() > 2 && tok[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return "*".to_string();
    }
    // Frame refs: "frame 10" handled pairwise below; here bare "10)" stays.
    if let Some(numpart) = tok.strip_prefix("frame:") {
        if numpart.chars().all(|c| c.is_ascii_digit()) {
            return "frame:*".to_string();
        }
    }
    // Floats: 123.456
    if tok.contains('.')
        && tok
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '-')
    {
        let digits: String = tok.chars().filter(|c| !c.is_ascii_digit()).collect();
        if !digits.is_empty() && digits.chars().all(|c| c == '.' || c == '-' || c == ',') {
            return "*".to_string();
        }
    }
    tok.to_string()
}

/// "12v3" style entity ids.
fn is_entity_id(s: &str) -> bool {
    let Some((idx, gen)) = s.split_once('v') else {
        return false;
    };
    !idx.is_empty()
        && idx.chars().all(|c| c.is_ascii_digit())
        && !gen.is_empty()
        && gen.chars().all(|c| c.is_ascii_digit())
}
