//! Dead letters in the memory adapter (issue #53, migration 0020): `insert_dead_letter` and `dead_letters`. The Postgres adapter is held
//! to the same answers by the generated sequences in `crates/adapters/selfhosted/tests/model`.
//!
//! The rules, written out:
//!
//! * `insert_dead_letter` is `true` for an id that is not there and `false` for one that is, and the row that is there stays as it
//!   was: the first write stands, whatever the second one carries;
//! * `dead_letters(n)` returns at most `n` rows, newest first by `received_at`; rows made at the same time are in descending order of
//!   `id`, compared byte by byte (so `dl-b` comes before `dl-a`, and both before `dl-B` and `dl-A`, which a case-insensitive order
//!   would interleave);
//! * `reset` leaves none.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::collections::BTreeMap;

use common::proptest_config;
use futures_executor::block_on;
use proptest::prelude::*;
use thrust_adapters_memory::MemoryDatabase;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_ports::*;

/// Ids that differ in case, so that byte order and case-insensitive order disagree: in bytes `A` < `B` < `a` < `b` < `c`.
const IDS: [&str; 5] = ["dl-a", "dl-A", "dl-b", "dl-B", "dl-c"];

#[derive(Debug, Clone)]
enum Op {
    /// `at` is one of three times, so that rows are often made at the same time and the id has to break the tie.
    Insert {
        id: usize,
        at: i64,
        tag: u8,
        attempts: Option<u32>,
        event: bool,
    },
    List {
        limit: u32,
    },
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    let op = prop_oneof![
        4 => (0..IDS.len(), 0..3i64, any::<u8>(), prop::option::of(1..=5u32), any::<bool>())
            .prop_map(|(id, at, tag, attempts, event)| Op::Insert { id, at, tag, attempts, event }),
        2 => (0..8u32).prop_map(|limit| Op::List { limit }),
    ];
    prop::collection::vec(op, 1..40)
}

fn letter(id: usize, at: i64, tag: u8, attempts: Option<u32>, event: bool) -> DeadLetter {
    DeadLetter {
        id: IDS[id].to_owned(),
        received_at: DateTime::from_timestamp(1_760_000_000 + at, 0).unwrap(),
        topic: "events".into(),
        event_id: event.then(|| EventId(Uuid::from_u128(u128::from(tag)))),
        body: format!("body {tag}"),
        error: format!("error {tag}"),
        attempts,
    }
}

proptest! {
    #![proptest_config(proptest_config(128))]

    #[test]
    fn model_dead_letters_follow_the_reference_model(ops in ops()) {
        let db = MemoryDatabase::new();
        let sys = SystemScope::mint();
        // the reference: a map from id to the row that was written first
        let mut model: BTreeMap<String, DeadLetter> = BTreeMap::new();

        for op in ops {
            match op {
                Op::Insert { id, at, tag, attempts, event } => {
                    let row = letter(id, at, tag, attempts, event);
                    let want = !model.contains_key(&row.id);
                    model.entry(row.id.clone()).or_insert_with(|| row.clone());
                    let got = block_on(db.insert_dead_letter(&sys, &row)).unwrap();
                    prop_assert_eq!(got, want, "insert {:?}", row.id);
                }
                Op::List { limit } => {
                    let mut want: Vec<DeadLetter> = model.values().cloned().collect();
                    want.sort_by(|a, b| b.received_at.cmp(&a.received_at).then_with(|| b.id.as_bytes().cmp(a.id.as_bytes())));
                    want.truncate(limit as usize);
                    let got = block_on(db.dead_letters(&sys, limit)).unwrap();
                    prop_assert_eq!(got, want, "list {}", limit);
                }
            }
        }

        // whatever was written, a reset leaves none (and the listing of everything is what the model holds)
        prop_assert_eq!(block_on(db.dead_letters(&sys, u32::MAX)).unwrap().len(), model.len());
        block_on(db.reset(&sys)).unwrap();
        prop_assert!(block_on(db.dead_letters(&sys, u32::MAX)).unwrap().is_empty(), "reset leaves none");
    }
}
