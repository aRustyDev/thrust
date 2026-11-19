//! Shared configuration for the unit tests in `src/` (from `.agents/skills/rust-test-logic/references/examples/`).

use proptest::test_runner::Config;

/// Proptest config whose case count can be raised (CI, nightly) or lowered (mutation runs) with
/// `PROPTEST_CASES`, while each test keeps its own default.
pub fn proptest_config(default_cases: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(default_cases);
    config_for(cases)
}

/// `cases` with proptest's reject budgets kept per case. proptest's limits are absolute (1,024 rejected inputs in all and 65,536 local
/// ones, which at its default 256 cases is 4 and 256 a case), so a test that discards a fixed share of its inputs (`prop_assume!`, a
/// filter) fits at 256 cases and gives up with "Too many rejects" at `PROPTEST_CASES=4096`, though nothing about the test changed
/// (found by the `deep` profile, status G-48). At 256 cases or fewer the limits are proptest's own, so the default loop is as it was.
fn config_for(cases: u32) -> Config {
    let proptest_default = Config::default();
    Config {
        cases,
        max_global_rejects: proptest_default.max_global_rejects.max(cases.saturating_mul(4)),
        max_local_rejects: proptest_default.max_local_rejects.max(cases.saturating_mul(256)),
        ..proptest_default
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    /// For any case count the budgets are at least proptest's own limits and at least 4 and 256 a case (saturating at `u32::MAX`),
    /// and the case count is the one asked for.
    #[rstest]
    fn config_for_keeps_the_reject_budgets_per_case(
        #[values(0, 1, 64, 255, 256, 257, 1024, 4096, 16_384, 65_536, 1_000_000, u32::MAX)] cases: u32,
    ) {
        let (config, proptest_default) = (config_for(cases), Config::default());
        assert_eq!(config.cases, cases);
        assert!(config.max_global_rejects >= proptest_default.max_global_rejects);
        assert!(config.max_local_rejects >= proptest_default.max_local_rejects);
        assert!(u64::from(config.max_global_rejects) >= (u64::from(cases) * 4).min(u64::from(u32::MAX)));
        assert!(u64::from(config.max_local_rejects) >= (u64::from(cases) * 256).min(u64::from(u32::MAX)));
    }

    /// At proptest's own default of 256 cases nothing changes: the limits are proptest's.
    #[test]
    fn config_for_at_the_default_cases_is_proptests_own() {
        let (config, proptest_default) = (config_for(256), Config::default());
        assert_eq!(config.max_global_rejects, proptest_default.max_global_rejects);
        assert_eq!(config.max_local_rejects, proptest_default.max_local_rejects);
    }
}
