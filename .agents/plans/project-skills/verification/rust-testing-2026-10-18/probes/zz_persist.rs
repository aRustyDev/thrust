// Probe: two properties that are false on purpose, one with proptest's default persistence and
// one through the zip's tests/common helper (WithSource).
mod common;
use proptest::prelude::*;

proptest! {
    #[test]
    fn prop_default_persistence(x in 0u32..1000) {
        prop_assert!(x < 10);
    }
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    #[test]
    fn prop_withsource_persistence(x in 0u32..1000) {
        prop_assert!(x < 10);
    }
}
