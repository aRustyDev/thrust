//! Migration inputs may be owned by a caller and freed after the ledger entries are made.
mod common;

use proptest::prelude::*;
use sha2::{Digest, Sha256};
use thrust_adapters_postgres::{carried, Migration};

proptest! {
    #![proptest_config(common::proptest_config(256))]

    #[test]
    fn prop_owned_migration_inputs_produce_owned_ledger_entries(name in "[a-z0-9_]{1,24}\\.sql", sql in ".{0,128}") {
        let expected_hash: String = Sha256::digest(sql.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        let entries = {
            let filename = name.clone();
            let source = sql.clone();
            let input = [Migration { file: &filename, sql: &source }];
            carried(&input)
        };
        prop_assert_eq!(entries.len(), 1);
        prop_assert_eq!(entries.into_iter().next().map(|entry| (entry.file, entry.sha256)), Some((name, expected_hash)));
    }
}
