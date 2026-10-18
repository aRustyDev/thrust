// TEMPLATE (rust-test-logic), adapted from reference-workspace.zip, 2026-10-18: copy to src/test_support.rs and declare it with `#[cfg(test)] pub(crate) mod test_support;`; nothing to substitute.
//! Shared test configuration.

use proptest::test_runner::Config;

/// Proptest config whose case count can be raised (CI, nightly) or lowered
/// (mutation runs) with `PROPTEST_CASES`, while each test keeps its own default.
pub fn proptest_config(default_cases: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_cases);
    Config {
        cases,
        ..Config::default()
    }
}
