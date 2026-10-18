// TEMPLATE (rust-test-logic), adapted from reference-workspace.zip, 2026-10-18: copy to tests/common/mod.rs; nothing to substitute. Each tests/*.rs file declares `mod common;`.
//! Shared helpers for integration tests (`tests/*.rs` files declare `mod common;`).

use proptest::test_runner::{Config, FailurePersistence, FileFailurePersistence};

/// Proptest config for integration tests.
/// * `PROPTEST_CASES` raises or lowers the case count without editing tests.
/// * Counterexamples are saved beside the test file (`tests/<file>.proptest-regressions`).
///   The default (`SourceParallel`) saves to the same place for files under `tests/`, but prints
///   `failed to find lib.rs or main.rs` on every failure; an explicit location avoids the noise.
/// * `PROPTEST_DISABLE_FAILURE_PERSISTENCE=1` turns saving off (use it for mutation runs);
///   proptest's default config honours it too, so this branch is belt-and-braces.
#[allow(dead_code)]
pub fn proptest_config(default_cases: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_cases);
    let failure_persistence: Option<Box<dyn FailurePersistence>> =
        if std::env::var_os("PROPTEST_DISABLE_FAILURE_PERSISTENCE").is_some() {
            None
        } else {
            Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions")))
        };
    Config { cases, failure_persistence, ..Config::default() }
}
