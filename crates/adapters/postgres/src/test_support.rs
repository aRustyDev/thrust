//! Shared property-test configuration.

use proptest::test_runner::Config;

pub fn proptest_config(default_cases: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|value| value.parse().ok()).unwrap_or(default_cases);
    Config { cases, ..Config::default() }
}
