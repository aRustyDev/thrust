//! Validate migration data before handing its SQL to a restricted preview connection.
//! This does not authorize a PR, establish artifact provenance, or validate SQL privileges.

use crate::{ledger, Migration};
use serde::Deserialize;

pub const MAX_WIRE_BYTES: usize = 32 * 1024 * 1024;
const MAX_FILES: usize = 256;
const MAX_FILE_BYTES: usize = 512 * 1024;
const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    version: u8,
    commit: String,
    files: Vec<File>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    name: String,
    sha256: String,
    sql: String,
}

fn lowercase_hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() >= 10
        && bytes.get(..4).is_some_and(|digits| digits.iter().all(u8::is_ascii_digit))
        && bytes.get(4) == Some(&b'_')
        && name.strip_suffix(".sql").and_then(|base| base.get(5..)).is_some_and(|part| {
            !part.is_empty() && part.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        })
}

impl Artifact {
    /// Parse a bounded artifact for an expected full commit SHA. The caller must authenticate
    /// that SHA and the source independently and recheck them before privileged use.
    pub fn parse(bytes: &[u8], expected_commit: &str) -> Option<Self> {
        if bytes.len() > MAX_WIRE_BYTES || !lowercase_hex(expected_commit, 40) {
            return None;
        }
        let artifact: Self = serde_json::from_slice(bytes).ok()?;
        if artifact.version != 1
            || artifact.commit != expected_commit
            || artifact.files.is_empty()
            || artifact.files.len() > MAX_FILES
        {
            return None;
        }
        let mut total = 0usize;
        let mut previous: Option<&str> = None;
        for file in &artifact.files {
            if !valid_name(&file.name)
                || !lowercase_hex(&file.sha256, 64)
                || file.sql.contains('\0')
                || file.sql.len() > MAX_FILE_BYTES
                || ledger::sha256_hex(&file.sql) != file.sha256
            {
                return None;
            }
            total = total.checked_add(file.sql.len())?;
            if total > MAX_TOTAL_BYTES {
                return None;
            }
            if let Some(old) = previous {
                if old >= file.name.as_str() || old.get(..4) == file.name.get(..4) {
                    return None;
                }
            }
            previous = Some(&file.name);
        }
        Some(artifact)
    }

    /// Borrow the validated files without copying their SQL.
    pub fn migrations(&self) -> Vec<Migration<'_>> {
        self.files.iter().map(|file| Migration { file: &file.name, sql: &file.sql }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::{json, Value};

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn artifact(sql: &str, version: u16) -> Value {
        json!({"version":1,"commit":COMMIT,"files":[{
            "name": format!("{version:04}_generated.sql"), "sha256":ledger::sha256_hex(sql), "sql":sql
        }]})
    }

    proptest! {
        #![proptest_config(crate::test_support::proptest_config(256))]
        #[test]
        fn prop_valid_artifact_round_trips(sql in "[^\\x00]{0,256}", version in 0u16..=9999) {
            let bytes = serde_json::to_vec(&artifact(&sql, version)).unwrap();
            let parsed = Artifact::parse(&bytes, COMMIT).unwrap();
            let files = parsed.migrations();
            prop_assert_eq!(files.len(), 1);
            prop_assert_eq!(files[0].sql, sql);
        }

        #[test]
        fn prop_tampered_artifact_is_rejected(sql in "[^\\x00]{0,128}", version in 0u16..=9999, mutation in 0u8..10) {
            let mut data = artifact(&sql, version);
            match mutation {
                0 => data["commit"] = json!("ffffffffffffffffffffffffffffffffffffffff"),
                1 => data["files"][0]["sql"] = json!(format!("{sql}x")),
                2 => data["files"][0]["name"] = json!("../escape.sql"),
                3 => data["version"] = json!(2),
                4 => data["files"][0]["extra"] = json!(true),
                5 => data["files"] = json!([]),
                6 => data["files"][0]["sha256"] = json!("0".repeat(64)),
                7 => { let file = data["files"][0].clone(); data["files"].as_array_mut().unwrap().push(file); }
                8 => { let mut file = data["files"][0].clone(); file["name"] = json!(format!("{version:04}_other.sql")); data["files"].as_array_mut().unwrap().push(file); }
                _ => data["files"][0]["sql"] = json!("\u{0}"),
            }
            prop_assert!(Artifact::parse(&serde_json::to_vec(&data).unwrap(), COMMIT).is_none());
        }

        #[test]
        fn prop_untrusted_artifact_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let _ = Artifact::parse(&bytes, COMMIT);
        }

        #[test]
        fn prop_rejects_noncanonical_commit_ids(position in 0usize..40, bad in prop_oneof![Just('G'), Just('A'), Just('_'), Just(' '), Just('/')]) {
            let mut commit = COMMIT.to_owned();
            commit.replace_range(position..position + 1, &bad.to_string());
            let mut data = artifact("", 1);
            data["commit"] = json!(commit);
            prop_assert!(Artifact::parse(&serde_json::to_vec(&data).unwrap(), &commit).is_none());
            for bad_length in ["", "123", "0123456789abcdef0123456789abcdef012345670"] {
                data["commit"] = json!(bad_length);
                prop_assert!(Artifact::parse(&serde_json::to_vec(&data).unwrap(), bad_length).is_none());
            }
        }

        #[test]
        fn prop_migration_name_grammar(name in "[a-z0-9_]{1,30}", number in 0u16..=9999, mutation in 0u8..7) {
            let mut data = artifact("", number);
            data["files"][0]["name"] = json!(format!("{number:04}_{name}.sql"));
            prop_assert!(Artifact::parse(&serde_json::to_vec(&data).unwrap(), COMMIT).is_some());
            let invalid = match mutation {
                0 => format!("x{number:04}_{name}.sql"),
                1 => format!("{number:04}-{name}.sql"),
                2 => format!("{number:04}_{name}-x.sql"),
                3 => format!("{number:04}_{name}.SQL"),
                4 => format!("{number:04}_{name}.sql/other"),
                5 => format!("{number:04}_.sql"),
                _ => format!("{number:04}_{name}é.sql"),
            };
            data["files"][0]["name"] = json!(invalid);
            prop_assert!(Artifact::parse(&serde_json::to_vec(&data).unwrap(), COMMIT).is_none());
        }

        #[test]
        fn prop_nul_is_rejected_even_with_matching_digest(before in "[^\\x00]{0,40}", after in "[^\\x00]{0,40}") {
            let sql = format!("{before}\0{after}");
            prop_assert!(Artifact::parse(&serde_json::to_vec(&artifact(&sql, 1)).unwrap(), COMMIT).is_none());
        }

        #[test]
        fn prop_files_must_be_in_version_order(low in 0u16..9999, offset in 1u16..=9999) {
            let high = low + (offset % (9999 - low)) + 1;
            let mut data = artifact("", low);
            let high_file = artifact("", high)["files"][0].clone();
            data["files"].as_array_mut().unwrap().push(high_file);
            prop_assert!(Artifact::parse(&serde_json::to_vec(&data).unwrap(), COMMIT).is_some());
            data["files"].as_array_mut().unwrap().reverse();
            prop_assert!(Artifact::parse(&serde_json::to_vec(&data).unwrap(), COMMIT).is_none());
        }
    }

    #[test]
    fn fuzz_replay_preview_artifact() {
        // Both seeds must exercise the decoder; an empty or entirely rejected corpus is not evidence.
        let valid = include_bytes!("../fuzz/seeds/preview_artifact/valid.json");
        let invalid = include_bytes!("../fuzz/seeds/preview_artifact/invalid.json");
        assert!(!valid.is_empty() && !invalid.is_empty());
        assert!(Artifact::parse(valid, COMMIT).is_some());
        assert!(Artifact::parse(invalid, COMMIT).is_none());
    }

    #[test]
    fn artifact_size_limits_spec() {
        // The producer's file-count and byte limits and the consumer's wire limit.
        let oversized_file = artifact(&"x".repeat(MAX_FILE_BYTES + 1), 1);
        assert!(Artifact::parse(&serde_json::to_vec(&oversized_file).unwrap(), COMMIT).is_none());
        let max_file = artifact(&"x".repeat(MAX_FILE_BYTES), 1);
        assert!(Artifact::parse(&serde_json::to_vec(&max_file).unwrap(), COMMIT).is_some());
        let files: Vec<Value> = (0..=MAX_FILES).map(|n| artifact("", n as u16)["files"][0].clone()).collect();
        let max_files = json!({"version":1,"commit":COMMIT,"files":&files[..MAX_FILES]});
        assert!(Artifact::parse(&serde_json::to_vec(&max_files).unwrap(), COMMIT).is_some());
        let too_many = json!({"version":1,"commit":COMMIT,"files":files});
        assert!(Artifact::parse(&serde_json::to_vec(&too_many).unwrap(), COMMIT).is_none());
        let too_large = vec![b' '; MAX_WIRE_BYTES + 1];
        assert!(Artifact::parse(&too_large, COMMIT).is_none());
        let mut max_wire = serde_json::to_vec(&artifact("", 1)).unwrap();
        max_wire.resize(MAX_WIRE_BYTES, b' ');
        assert!(Artifact::parse(&max_wire, COMMIT).is_some());
        let files: Vec<Value> = (0..9).map(|n| artifact(&"x".repeat(MAX_FILE_BYTES), n)["files"][0].clone()).collect();
        let max_sql = json!({"version":1,"commit":COMMIT,"files":&files[..8]});
        assert!(Artifact::parse(&serde_json::to_vec(&max_sql).unwrap(), COMMIT).is_some());
        let too_much_sql = json!({"version":1,"commit":COMMIT,"files":files});
        assert!(Artifact::parse(&serde_json::to_vec(&too_much_sql).unwrap(), COMMIT).is_none());
    }
}
