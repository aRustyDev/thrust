//! The fuzz harness of the one parser of outside text in `thrust-contracts` (rust-test-fuzz): [`DeadLetterMessage::parse`], which reads
//! what arrives on the events dead-letter queue, where any producer's message can land. The cargo-fuzz target in `fuzz/fuzz_targets/`
//! calls it; the `fuzz_replay_` tests below run every committed seed and regression through it on each `cargo nextest run`. Compiled only
//! for tests and fuzz builds, never shipped.

#![allow(
    clippy::panic,
    reason = "a fuzz harness reports a broken invariant by panicking: that is what the fuzzer looks for"
)]

use crate::{DeadLetterMessage, Envelope, DEAD_LETTER_MESSAGE_KIND};

/// A message body as the dead-letter queue delivers it. Must never panic. When it is one of ours, it must read back the same after
/// being written out again; and whatever else it is (an envelope included), it must not be taken for one. Returns whether it was one.
pub fn dead_letter_message(data: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(data) else { return false };
    match DeadLetterMessage::parse(text) {
        Some(m) => {
            assert_eq!(m.kind, DEAD_LETTER_MESSAGE_KIND, "only the marked shape is taken");
            let again = m.to_text().unwrap_or_default();
            assert_eq!(DeadLetterMessage::parse(&again), Some(m), "write -> parse must give the same message");
            true
        }
        None => {
            // not one of ours; if it is an envelope, which is what a producer sends, it is plainly not
            if let Ok(env) = serde_json::from_str::<Envelope>(text) {
                assert!(DeadLetterMessage::parse(&serde_json::to_string(&env).unwrap_or_default()).is_none());
            }
            false
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "test code")]
mod tests {
    use super::dead_letter_message;
    use std::panic::catch_unwind;
    use std::path::{Path, PathBuf};

    /// Regular files directly under `dir`, sorted. A missing directory gives an empty list; `replay` makes that an error.
    fn files_in(dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut files: Vec<PathBuf> = entries.map(|e| e.unwrap().path()).filter(|p| p.is_file()).collect();
        files.sort();
        files
    }

    fn run_one(input: &Path, run: fn(&[u8]) -> bool) -> bool {
        let data = std::fs::read(input).unwrap();
        match catch_unwind(|| run(&data)) {
            Ok(decoded) => decoded,
            Err(_) => panic!("fuzz input {} panicked", input.display()),
        }
    }

    /// Seeds must exist on their own and at least one must decode; regressions may be absent.
    fn replay(seeds: &Path, regressions: &Path, run: fn(&[u8]) -> bool) {
        let seed_files = files_in(seeds);
        assert!(!seed_files.is_empty(), "no seed inputs found in {}", seeds.display());
        let decoded = seed_files.iter().filter(|f| run_one(f, run)).count();
        assert!(
            decoded > 0,
            "no seed in {} reaches the success branch (written as text instead of bytes?)",
            seeds.display()
        );
        for input in files_in(regressions) {
            run_one(&input, run);
        }
    }

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz")
    }

    #[test]
    fn fuzz_replay_dead_letter_message() {
        replay(
            &root().join("seeds/dead_letter_message"),
            &root().join("regressions/dead_letter_message"),
            dead_letter_message,
        );
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Control: an empty seed directory, or seeds that never decode, fail the replay.
    #[test]
    fn fuzz_replay_control_vacuous_inputs_fail() {
        let dir = scratch_dir("contracts-fuzz-replay-vacuous");
        let on_empty = catch_unwind(|| replay(&dir, &dir, dead_letter_message));
        std::fs::write(dir.join("text-not-a-message"), b"not json at all").unwrap();
        let on_text = catch_unwind(|| replay(&dir, &dir, dead_letter_message));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(on_empty.is_err(), "an empty seed directory must fail the replay");
        assert!(on_text.is_err(), "seeds that never decode must fail the replay");
    }

    /// Control: the committed valid seed really reaches the success branch, so the invariants run; a seed with its marker changed
    /// does not (it is some other producer's JSON), and neither does the envelope seed.
    #[test]
    fn fuzz_replay_control_the_valid_seed_decodes_and_a_changed_one_does_not() {
        let seeds = root().join("seeds/dead_letter_message");
        let valid = std::fs::read(seeds.join("valid")).unwrap();
        assert!(dead_letter_message(&valid), "the valid seed must decode");

        let text = String::from_utf8(valid).unwrap();
        let changed = text.replace("thrust.dead-letter.v1", "thrust.dead-letter.v2");
        assert_ne!(changed, text, "the marker is in the seed");
        assert!(!dead_letter_message(changed.as_bytes()), "a seed with another marker must not decode");

        let envelope = std::fs::read(seeds.join("rejected-envelope")).unwrap();
        assert!(!dead_letter_message(&envelope), "an envelope is not a dead letter message");
    }
}
