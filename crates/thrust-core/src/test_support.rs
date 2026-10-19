//! Shared configuration for the unit tests in `src/` (from `.agents/skills/rust-test-logic/references/examples/`).

use proptest::test_runner::Config;

/// Proptest config whose case count can be raised (CI, nightly) or lowered (mutation runs) with
/// `PROPTEST_CASES`, while each test keeps its own default.
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
