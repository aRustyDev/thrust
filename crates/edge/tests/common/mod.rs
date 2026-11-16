//! Shared helpers for the `thrust-edge` tests: proptest configuration, a one-poll executor for the fake flags, deterministic RSA
//! keys and a token builder for the Access tests, and fake Flagship answers.
#![allow(dead_code)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

use std::future::Future;
use std::sync::OnceLock;
use std::task::{Context, Poll, Waker};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use proptest::test_runner::{Config, FailurePersistence, FileFailurePersistence};
use rand::SeedableRng as _;
use rsa::traits::PublicKeyParts as _;
use rsa::{Pkcs1v15Sign, RsaPrivateKey};
use sha2::{Digest as _, Sha256};
use thrust_edge::route::{EvalContext, FlagError, Flags};

/// Proptest config (from `.agents/skills/rust-test-logic/references/examples/`): `PROPTEST_CASES` overrides the count,
/// counterexamples are saved beside the test file, `PROPTEST_DISABLE_FAILURE_PERSISTENCE=1` turns saving off (mutation runs).
pub fn proptest_config(default_cases: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(default_cases);
    let failure_persistence: Option<Box<dyn FailurePersistence>> =
        if std::env::var_os("PROPTEST_DISABLE_FAILURE_PERSISTENCE").is_some() {
            None
        } else {
            Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions")))
        };
    Config { cases, failure_persistence, ..Config::default() }
}

/// Runs a future that never waits (the fake flags answer at once). A fake that returned `Pending` would be a test bug.
pub fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    match f.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("a fake Flagship answer waited"),
    }
}

/// A fixed key for the pin (32 bytes, the minimum), and another one for forgeries.
pub const PIN_KEY: &[u8] = b"0123456789abcdef0123456789abcdef";
pub const OTHER_PIN_KEY: &[u8] = b"fedcba9876543210fedcba9876543210";

/// Two RSA-2048 key pairs from fixed seeds: `rsa_keys().0` is the team's, `.1` an attacker's. Generated once per test binary.
pub fn rsa_keys() -> &'static (RsaPrivateKey, RsaPrivateKey) {
    static KEYS: OnceLock<(RsaPrivateKey, RsaPrivateKey)> = OnceLock::new();
    KEYS.get_or_init(|| {
        let a = RsaPrivateKey::new(&mut rand::rngs::StdRng::seed_from_u64(1), 2048).unwrap();
        let b = RsaPrivateKey::new(&mut rand::rngs::StdRng::seed_from_u64(2), 2048).unwrap();
        (a, b)
    })
}

pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// A JWKS document with `key` under `kid`, as Cloudflare's `/cdn-cgi/access/certs` lays it out (`keys` plus the PEM members).
pub fn jwks_doc(kid: &str, key: &RsaPrivateKey) -> String {
    let public = key.to_public_key();
    serde_json::json!({
        "keys": [{
            "kid": kid, "kty": "RSA", "alg": "RS256", "use": "sig",
            "e": b64(&public.e().to_bytes_be()), "n": b64(&public.n().to_bytes_be()),
        }],
        "public_cert": { "kid": kid, "cert": "-----BEGIN CERTIFICATE-----\n...\n-----END CERTIFICATE-----" },
        "public_certs": [],
    })
    .to_string()
}

/// `header.payload.signature`, signed RS256 with `key` (or with no signature when `key` is `None`).
pub fn token(header: &serde_json::Value, claims: &serde_json::Value, key: Option<&RsaPrivateKey>) -> String {
    let signed = format!("{}.{}", b64(header.to_string().as_bytes()), b64(claims.to_string().as_bytes()));
    let signature = match key {
        Some(k) => k.sign(Pkcs1v15Sign::new::<Sha256>(), &Sha256::digest(signed.as_bytes())).unwrap(),
        None => Vec::new(),
    };
    format!("{signed}.{}", b64(&signature))
}

/// What a fake Flagship answers.
#[derive(Clone, Debug)]
pub struct FakeFlags {
    pub routing: Result<bool, FlagError>,
    /// alias key -> answer; an absent key answers "" (no such flag).
    pub aliases: Vec<(String, Result<String, FlagError>)>,
    /// target key -> (the kinds allowed, the value served to them); absent or denied answers `{}`.
    pub targets: Vec<(String, Vec<&'static str>, Result<serde_json::Value, FlagError>)>,
}

impl Flags for FakeFlags {
    async fn routing(&self) -> Result<bool, FlagError> {
        self.routing
    }

    async fn alias(&self, key: &str) -> Result<String, FlagError> {
        self.aliases.iter().find(|(k, _)| k == key).map_or(Ok(String::new()), |(_, v)| v.clone())
    }

    async fn target(&self, key: &str, ctx: &EvalContext) -> Result<serde_json::Value, FlagError> {
        match self.targets.iter().find(|(k, _, _)| k == key) {
            Some((_, kinds, value)) if kinds.contains(&ctx.kind) => value.clone(),
            Some((_, _, Err(e))) => Err(*e),
            _ => Ok(serde_json::json!({})),
        }
    }
}
