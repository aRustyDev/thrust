// TEMPLATE (rust-test-fuzz): copy to src/fuzz_harness.rs; `frame` is an example codec, replace it with yours.
//! Shared fuzz harness. The cargo-fuzz and afl targets call `parse_frame`; the replay tests below run
//! every saved input through it on each `cargo nextest run`.

use crate::frame::{self, Frame, FrameError};

/// Must never panic. When decoding succeeds, re-encoding must reproduce the input.
/// Returns whether the input decoded: fuzz targets ignore it, the replay uses it to prove that
/// the seeds reach the invariant.
pub fn parse_frame(data: &[u8]) -> bool {
    check(data, frame::decode)
}

fn check(data: &[u8], decode: fn(&[u8]) -> Result<Frame, FrameError>) -> bool {
    match decode(data) {
        Ok(f) => {
            let re = frame::encode(&f).expect("a decoded frame must re-encode");
            assert_eq!(re, data, "decode -> encode must be lossless");
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{check, parse_frame};
    use crate::frame::{self, Frame, FrameError};
    use std::panic::catch_unwind;
    use std::path::{Path, PathBuf};

    /// Regular files directly under `dir`, sorted so the order (and the first failure) is stable.
    /// A missing directory gives an empty list; `replay` decides that an empty seed list is an error.
    fn files_in(dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .map(|entry| entry.unwrap().path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        files
    }

    /// Runs one input; a panic fails the test and names the file.
    fn run_one(input: &Path, run: fn(&[u8]) -> bool) -> bool {
        let data = std::fs::read(input).unwrap();
        match catch_unwind(|| run(&data)) {
            Ok(decoded) => decoded,
            Err(_) => panic!("fuzz input {} panicked", input.display()),
        }
    }

    /// Seeds must exist, be non-empty on their own, and at least one must decode.
    /// Regressions may be absent or empty.
    fn replay(seeds: &Path, regressions: &Path, run: fn(&[u8]) -> bool) {
        let seed_files = files_in(seeds);
        assert!(!seed_files.is_empty(), "no seed inputs found in {}", seeds.display());
        let mut decoded = 0;
        for input in &seed_files {
            if run_one(input, run) {
                decoded += 1;
            }
        }
        assert!(
            decoded > 0,
            "no seed in {} reaches the success branch (written as text instead of bytes?)",
            seeds.display()
        );
        for input in files_in(regressions) {
            run_one(&input, run);
        }
    }

    /// Seed corpus plus minimized crash reproducers, replayed deterministically.
    #[test]
    fn fuzz_replay_parse_frame() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz");
        replay(
            &root.join("seeds/parse_frame"),
            &root.join("regressions/parse_frame"),
            parse_frame,
        );
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Control: an empty seed directory, or seeds that never decode, must FAIL the replay.
    /// (Adapt the text file to any input your codec rejects.)
    #[test]
    fn fuzz_replay_control_vacuous_inputs_fail() {
        let dir = scratch_dir("fuzz-replay-vacuous");
        let on_empty = catch_unwind(|| replay(&dir, &dir, parse_frame));
        std::fs::write(dir.join("text-not-bytes"), br"\x07\x00\x00").unwrap();
        let on_text = catch_unwind(|| replay(&dir, &dir, parse_frame));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(on_empty.is_err(), "an empty seed directory must fail the replay");
        assert!(on_text.is_err(), "seeds that never decode must fail the replay");
    }

    /// Control: a codec that drops a payload byte must turn the replay red.
    /// (Adapt `lossy` to break your codec's round trip.)
    #[test]
    fn fuzz_replay_control_broken_codec_fails() {
        fn lossy(data: &[u8]) -> Result<Frame, FrameError> {
            let mut f = frame::decode(data)?;
            f.payload.pop();
            Ok(f)
        }
        let dir = scratch_dir("fuzz-replay-lossy");
        std::fs::write(dir.join("valid"), [1u8, 0, 3, b'a', b'b', b'c']).unwrap();
        let outcome = catch_unwind(|| replay(&dir, &dir, |data| check(data, lossy)));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(outcome.is_err(), "a lossy codec must fail the replay");
    }
}
