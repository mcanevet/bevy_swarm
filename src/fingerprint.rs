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
        // FX7: merge_keys redact the key's (textual) value BEFORE
        // normalization — only messages CONTAINING the key are merged;
        // others are fingerprinted normally (no blanket "[merged]"
        // collapse). reject_keys split on the key's VALUE extracted
        // from the RAW message (pre-normalization), so two failures
        // with different values for a reject key stay distinct.
        let redacted = self.redact_merge_keys(message);
        let normalized = self.normalize(&redacted);
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
        for loc in location {
            buf.push('\u{1}');
            buf.push_str(loc);
        }
        buf.push('\u{1}');
        buf.push_str(&normalized);
        for key in &self.reject_keys {
            if let Some(value) = extract_key_value(message, key) {
                // Split on the raw value (pre-normalization): numeric
                // values would otherwise be wildcarded to "*" and never
                // split anything.
                buf.push('\u{1}');
                buf.push_str(&value);
            }
        }
        Fingerprint(format!("{:016x}", fnv1a(buf.as_bytes())))
    }

    /// FX7: replace each merge key's VALUE with a marker. Only keys
    /// PRESENT in the message are redacted — a message without any
    /// merge key keeps its full normalized body.
    fn redact_merge_keys(&self, raw: &str) -> String {
        let mut out = raw.to_string();
        for key in &self.merge_keys {
            let Some(pos) = out.find(key.as_str()) else {
                continue;
            };
            let after = pos + key.len();
            let rest = &out[after..];
            let sep_len = usize::from(rest.starts_with('=') || rest.starts_with(':'));
            let val_end = rest[sep_len..]
                .find(char::is_whitespace)
                .map(|i| sep_len + i)
                .unwrap_or(rest.len());
            out = format!("{}[merged:{}]{}", &out[..pos], key, &rest[val_end..]);
        }
        out
    }
}

/// Extract a reject key's VALUE from the RAW message (pre-
/// normalization): find the key, skip one separator ('=' / ':' /
/// whitespace) and take the value up to the next whitespace.
/// Returns None when the key is absent.
fn extract_key_value(raw: &str, key: &str) -> Option<String> {
    let pos = raw.find(key)?;
    let rest = &raw[pos + key.len()..];
    let sep_len = usize::from(rest.starts_with('=') || rest.starts_with(':'));
    let value: String = rest[sep_len..]
        .chars()
        .take_while(|c| !c.is_whitespace())
        .collect();
    Some(value)
}

/// Default normalizer for v0.2 (no special reject/merge keys yet).
pub fn default_normalizer() -> Normalizer {
    Normalizer::default()
}

/// FX7: config-loaded normalizer. Reads `tests/swarm/config.json`
/// (U1 conventions path) when present; falls back to the default.
/// Structure: {"fingerprint_rules": {...}} sibling keys are tolerated;
/// reject/merge keys are read from an optional "fingerprints" object:
/// {"reject_keys": [...], "merge_keys": [...]}.
pub fn config_normalizer() -> Normalizer {
    let path = crate::conventions::config_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Normalizer::default();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Normalizer::default();
    };
    let mut n = Normalizer::default();
    if let Some(reject) = v
        .get("fingerprints")
        .and_then(|f| f.get("reject_keys"))
        .and_then(|k| k.as_array())
    {
        n.reject_keys = reject
            .iter()
            .filter_map(|k| k.as_str().map(String::from))
            .collect();
    }
    if let Some(merge) = v
        .get("fingerprints")
        .and_then(|f| f.get("merge_keys"))
        .and_then(|k| k.as_array())
    {
        n.merge_keys = merge
            .iter()
            .filter_map(|k| k.as_str().map(String::from))
            .collect();
    }
    n
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
    // Entity(12v3) — Bevy's Debug format for entity ids.
    if let Some(inner) = tok
        .strip_prefix("Entity(")
        .and_then(|s| s.strip_suffix(')'))
    {
        if is_entity_id(inner) {
            return "Entity(*)".to_string();
        }
    }
    // StableId(N) — sequence-stable but run-relative.
    if let Some(inner) = tok
        .strip_prefix("StableId(")
        .and_then(|s| s.strip_suffix(')'))
    {
        if inner.chars().all(|c| c.is_ascii_digit()) {
            return "StableId(*)".to_string();
        }
    }
    // seed=N
    if let Some(numpart) = tok.strip_prefix("seed=") {
        if numpart.chars().all(|c| c.is_ascii_digit()) {
            return "seed=*".to_string();
        }
    }
    // Floats adjacent to punctuation: "123.456", "(123.456,", "123.456)".
    // Trim leading/trailing non-alphanumeric noise, then test the core.
    let core = tok.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-');
    if core.contains('.')
        && core
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '-')
    {
        let digits: String = core.chars().filter(|c| !c.is_ascii_digit()).collect();
        if !digits.is_empty() && digits.chars().all(|c| c == '.' || c == '-') {
            return "*".to_string();
        }
    }
    // Bare integers with punctuation noise ("123,", "123)") — only
    // wildcard when the whole token is numeric-plus-noise (a mixed
    // token like "score3" stays).
    if !core.is_empty() && core.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return "*".to_string();
    }
    // key=<number> ("value=123.456", "score=3"): wildcard the numeric
    // suffix but keep the key.
    for sep in ['=', ':'] {
        if let Some((head, tail)) = tok.split_once(sep) {
            let tail_core =
                tail.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-');
            if !tail_core.is_empty()
                && tail_core
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
            {
                return format!("{head}{sep}*");
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fx7_normalize_table_driven() {
        // FX7 T1: table-driven unit tests for each normalizer gap.
        let n = Normalizer::default();
        let cases = vec![
            // Floats adjacent to punctuation
            ("(20000.5,)", "*"),
            ("123.456)", "*"),
            ("value=123.456", "value=*"),
            // Bare ints with punctuation
            ("123,", "*"),
            ("123)", "*"),
            ("score3", "score3"), // mixed token stays
            // Seeds
            ("seed=12345", "seed=*"),
            // Frame refs
            ("frame 10", "frame *"),
            ("frame:123", "frame:*"),
            // Entity ids
            ("Entity(12v3)", "Entity(*)"),
            ("entity:123v4", "entity:*"),
            // StableIds
            ("StableId(42)", "StableId(*)"),
            // Hex addresses
            ("0xdeadbeef", "*"),
        ];
        for (input, expected) in cases {
            let got = n.normalize(input);
            assert_eq!(
                got, expected,
                "normalize({input}) = {got} (expected {expected})"
            );
        }
    }

    #[test]
    fn fx7_fingerprint_stability_across_seeds() {
        // FX7 T1: same bug on different entity/seed -> one fingerprint.
        let n = Normalizer::default();
        let fp1 = n.fingerprint(
            "violation",
            "panic",
            &["world".to_string()],
            "entity:12v3 panicked at seed=1",
        );
        let fp2 = n.fingerprint(
            "violation",
            "panic",
            &["world".to_string()],
            "entity:45v7 panicked at seed=2",
        );
        assert_eq!(
            fp1, fp2,
            "different entity/seed should yield same fingerprint"
        );
    }

    #[test]
    fn fx7_fingerprint_stability_different_entities() {
        // FX7 T1: leak on two different spawned entities -> one fingerprint.
        let n = Normalizer::default();
        let fp1 = n.fingerprint(
            "violation",
            "leak",
            &["world".to_string()],
            "entity:12v3 leaked resource",
        );
        let fp2 = n.fingerprint(
            "violation",
            "leak",
            &["world".to_string()],
            "entity:45v7 leaked resource",
        );
        assert_eq!(fp1, fp2, "different entities should yield same fingerprint");
    }
}
