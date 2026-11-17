//! The workload itself, hermetic (no file, no network, injected time and ids), in the default loop: a profile of a workload that was
//! refused would be a profile of error paths. Generated mixes, so every kind of request is held to answering, in any proportion.

use proptest::prelude::*;
mod common;

use common::world;
use thrust_profiling::{run, Mix};

fn config(default_cases: u32) -> proptest::test_runner::Config {
    use proptest::test_runner::{Config, FailurePersistence, FileFailurePersistence};
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(default_cases);
    let failure_persistence: Option<Box<dyn FailurePersistence>> =
        if std::env::var_os("PROPTEST_DISABLE_FAILURE_PERSISTENCE").is_some() {
            None
        } else {
            Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions")))
        };
    Config { cases, failure_persistence, ..Config::default() }
}

proptest! {
    #![proptest_config(config(16))]

    /// Every request of a mix is made and answered with a 2xx, except updates when nothing was created to update: those are
    /// counted as refused, one each, so a mix that cannot do what it says cannot pass as a profile.
    #[test]
    fn prop_every_request_of_a_mix_is_answered(creates in 0usize..12, lists in 0usize..6, searches in 0usize..6, updates in 0usize..12) {
        let mix = Mix { creates, lists, searches, updates };
        let out = run(&world(), &mix, 0);
        prop_assert_eq!(out.requests, mix.requests());
        if creates == 0 && updates > 0 {
            prop_assert_eq!(out.refused.len(), updates);
        } else {
            prop_assert!(out.refused.is_empty(), "refused: {:?}", out.refused);
        }
    }
}

/// The mix the `prof_` tests profile is answered in full.
#[test]
fn the_profiled_mix_is_answered() {
    // the one mix that is profiled, Mix::PROFILE (the generator above draws smaller ones)
    let out = run(&world(), &Mix::PROFILE, 0);
    assert_eq!(out.requests, Mix::PROFILE.requests());
    assert!(out.refused.is_empty(), "refused: {:?}", out.refused);
}
