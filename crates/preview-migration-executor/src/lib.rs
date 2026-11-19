//! Closed-by-default preview migration executor.
//!
//! Request authentication and artifact provenance must be enforced by the trusted caller and
//! Cloudflare Access before any route is enabled. Commit metadata in an artifact is not proof.

use thrust_adapters_postgres::preview_artifact::{Artifact, MAX_WIRE_BYTES};

#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    pub number: u32,
    pub commit: String,
    pub database: String,
}

impl Config {
    /// Only a trusted deployment may enable and set these per-revision values.
    pub fn parse(enabled: &str, number: &str, commit: &str, database: &str) -> Option<Self> {
        if enabled != "true"
            || number.is_empty()
            || number.len() > 9
            || number.starts_with('0')
            || !number.bytes().all(|b| b.is_ascii_digit())
            || commit.len() != 40
            || !commit.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || database.is_empty()
            || database.len() > 63
            || !database.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return None;
        }
        Some(Self { number: number.parse().ok()?, commit: commit.to_owned(), database: database.to_owned() })
    }

    pub fn artifact(&self, body: &[u8]) -> Option<Artifact> {
        Artifact::parse(body, &self.commit)
    }

    pub fn matches_binding(&self, user: &str, database: &str) -> bool {
        user == format!("pr_{}", self.number) && database == self.database
    }
}

pub fn is_route(method: &str, path: &str) -> bool {
    method == "POST" && path == "/v1/migrate"
}

/// Add a chunk without accepting an oversized body or retaining a partial oversized chunk.
pub fn append_bounded(body: &mut Vec<u8>, chunk: &[u8]) -> bool {
    if body.len().saturating_add(chunk.len()) > MAX_WIRE_BYTES {
        return false;
    }
    body.extend_from_slice(chunk);
    true
}

#[cfg(target_arch = "wasm32")]
mod runtime;

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    proptest! {
        #[test]
        fn prop_configuration_is_bound_to_a_single_preview(number in 1u32..=999_999_999, other in 1u32..=999_999_999) {
            let config = Config::parse("true", &number.to_string(), SHA, "thrust").unwrap();
            let expected_user = format!("pr_{number}");
            let other_user = format!("pr_{other}");
            let padded = format!("0{number}");
            prop_assert!(config.matches_binding(&expected_user, "thrust"));
            prop_assert!(!config.matches_binding(&expected_user, "other"));
            if other != number { prop_assert!(!config.matches_binding(&other_user, "thrust")); }
            prop_assert!(Config::parse("false", &number.to_string(), SHA, "thrust").is_none());
            prop_assert!(Config::parse("true", &padded, SHA, "thrust").is_none());
        }

        #[test]
        fn prop_invalid_configuration_fails_closed(number in 1u32..=999_999_999, position in 0usize..40) {
            let mut sha = SHA.to_owned();
            sha.replace_range(position..position + 1, "G");
            prop_assert!(Config::parse("true", &number.to_string(), &sha, "thrust").is_none());
            prop_assert!(Config::parse("true", &number.to_string(), SHA, "thrust-public").is_none());
            prop_assert!(Config::parse("TRUE", &number.to_string(), SHA, "thrust").is_none());
        }

        #[test]
        fn prop_only_exact_route_is_accepted(path in ".{0,100}", method in ".{0,16}") {
            prop_assert!(is_route("POST", "/v1/migrate"));
            prop_assert_eq!(is_route(&method, &path), method == "POST" && path == "/v1/migrate");
        }

        #[test]
        fn prop_bounded_chunks_preserve_bytes(chunks in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..100), 0..20)) {
            let mut body = Vec::new();
            let expected: Vec<u8> = chunks.iter().flatten().copied().collect();
            for chunk in chunks { prop_assert!(append_bounded(&mut body, &chunk)); }
            prop_assert_eq!(body, expected);
        }
    }

    #[test]
    fn artifact_and_wire_limit_spec() {
        // The artifact producer's committed valid seed carries this exact SHA.
        let config = Config::parse("true", "1", SHA, "thrust").unwrap();
        let seed = include_bytes!("../../adapters/postgres/fuzz/seeds/preview_artifact/valid.json");
        assert!(config.artifact(seed).is_some());
        let other = Config::parse("true", "1", "ffffffffffffffffffffffffffffffffffffffff", "thrust").unwrap();
        assert!(other.artifact(seed).is_none());
        let mut body = vec![0; MAX_WIRE_BYTES];
        assert!(!append_bounded(&mut body, &[1]));
        assert_eq!(body.len(), MAX_WIRE_BYTES);
    }
}
