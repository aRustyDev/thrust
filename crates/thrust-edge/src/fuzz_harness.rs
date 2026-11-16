//! Fuzz harnesses for the two parsers of outside bytes on the edge (rust-test-fuzz): the pin cookie and the Access token. The
//! cargo-fuzz targets in `fuzz/fuzz_targets/` call these; the `fuzz_replay_` tests below run every committed seed and regression
//! through them on each `cargo nextest run`. Compiled only for tests and fuzz builds, never shipped.

#![allow(
    clippy::panic,
    reason = "a fuzz harness reports a broken invariant by panicking: that is what the fuzzer looks for"
)]

use crate::access::{self, Expect, Jwks};
use crate::pin;

/// The key the pin seeds were sealed with (fuzz/seeds/pin_open/valid). Not a secret: it seals nothing real.
pub const PIN_KEY: &[u8] = b"thrust-edge-fuzz-harness-key-0123";
/// What the token seeds were issued for (fuzz/seeds/access_verify/valid).
pub const ISSUER: &str = "https://fuzz.cloudflareaccess.com";
pub const AUDIENCE: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
pub const NOW: u64 = 1_800_000_000;
/// The public half of the throwaway key that signed the token seed; its private half was never written down.
const JWKS: &str = r#"{"keys":[{"kid":"fuzz","kty":"RSA","alg":"RS256","use":"sig","e":"AQAB","n":"pH7Ft0BwmyV9vBXYeShatIyunV4LAylDF9FDB37YMIsITzhVKr-q6Dh5GiJevtCcXhPX3uGMlC2dCqfZqLkV-FKDQ6uGJiWDuMsMujJwSwXWptT7jo_mOztHDRUBPW-zSl2OlBYJnfJ_1n6Sh5kNK5laEBilRCeAaTI3uYUgJzw8mvUoVP-d8xp4eCEBFmQQaTQWCjJbgBGm97u800tPvyAn1zvOwpxAEQmfHXiMvYJIBUp5HICvPNxoiDc-zGE-2-OdAXbJurI3BdsHv-vBnTBSkqidcE1GFcRJ6vUQ2UewspAbhGu_z55pq5gg8kQz7rDuaivQmK6O1X_mt5RFKw"}]}"#;

/// A pin cookie value, or a Cookie header holding one. Must never panic. When it opens, sealing what it opened must open to the
/// same pin (the value may differ in its JSON's spelling, so the round trip is checked on the decoded pin, not the bytes).
pub fn pin_open(data: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(data) else { return false };
    let value = match pin::from_cookie_header(s) {
        Some(v) => {
            assert!(s.contains(v) && !v.contains(';'), "a cookie value is a piece of its header");
            v
        }
        None => s,
    };
    match pin::open(value, PIN_KEY, 0) {
        Ok(p) => {
            let again = pin::seal(&p, PIN_KEY).unwrap_or_default();
            assert_eq!(pin::open(&again, PIN_KEY, 0).ok(), Some(p), "open -> seal -> open must give the same pin");
            true
        }
        Err(_) => false,
    }
}

fn jwks() -> Jwks {
    access::parse_jwks(JWKS.as_bytes()).unwrap_or_else(|e| panic!("the harness JWKS must parse: {e:?}"))
}

/// An Access token. Must never panic; the answer is a function of the input alone (verifying twice agrees), and an accepted
/// token names a principal.
pub fn access_verify(data: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(data) else { return false };
    let keys = jwks();
    let expect = Expect { issuer: ISSUER, audience: AUDIENCE, now: NOW };
    let first = access::verify(s, &keys, &expect);
    assert_eq!(first, access::verify(s, &keys, &expect), "verification must be deterministic");
    match first {
        Ok(who) => {
            assert!(pin::valid_principal(&who.principal), "an accepted token names a principal");
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "test code")]
mod tests {
    use super::{access_verify, pin_open};
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
    fn fuzz_replay_pin_open() {
        replay(&root().join("seeds/pin_open"), &root().join("regressions/pin_open"), pin_open);
    }

    #[test]
    fn fuzz_replay_access_verify() {
        replay(&root().join("seeds/access_verify"), &root().join("regressions/access_verify"), access_verify);
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
        let dir = scratch_dir("edge-fuzz-replay-vacuous");
        let on_empty = catch_unwind(|| replay(&dir, &dir, pin_open));
        std::fs::write(dir.join("text-not-a-pin"), b"v1.not.a-pin").unwrap();
        let on_text = catch_unwind(|| replay(&dir, &dir, pin_open));
        let on_text_token = catch_unwind(|| replay(&dir, &dir, access_verify));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(on_empty.is_err(), "an empty seed directory must fail the replay");
        assert!(on_text.is_err() && on_text_token.is_err(), "seeds that never decode must fail the replay");
    }

    /// Control: the committed valid seeds really reach the success branch, so the invariants run (a seed altered by one byte
    /// does not).
    #[test]
    fn fuzz_replay_control_the_valid_seeds_decode_and_a_changed_one_does_not() {
        for (target, run) in [("pin_open", pin_open as fn(&[u8]) -> bool), ("access_verify", access_verify)] {
            let valid = std::fs::read(root().join("seeds").join(target).join("valid")).unwrap();
            assert!(run(&valid), "{target}: the valid seed must decode");
            let mut changed = valid.clone();
            let mid = changed.len() / 2;
            changed[mid] = if changed[mid] == b'A' { b'B' } else { b'A' };
            assert!(!run(&changed), "{target}: a changed seed must not decode");
        }
    }
}
