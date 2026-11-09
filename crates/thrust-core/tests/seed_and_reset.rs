//! The two test-support use-cases that #58's seed needs (`thrust_core::seed` with a fixture's `profiles`, and `thrust_core::reset`),
//! against the memory adapters. What the database does with a fixture's work items, and what a reset leaves, is the adapters' own
//! tests (`seed_and_reset.rs` in `thrust-adapters-memory`, `pg_adapter.rs` in `thrust-adapters-selfhosted`).
//!
//! What must hold: a profile of the fixture has the names it was given and the defaults every account starts with; a user that has a
//! profile already keeps its row (its id) and takes the new names; a user with no entry in `profiles` is not given one; `reset`
//! reaches the database, and what it empties can be seeded again.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

const PERSON: u128 = 0xE1;

fn person() -> UserId {
    UserId(uuid(PERSON))
}

fn fixture(first: &str, last: &str, display: &str) -> SeedFixture {
    SeedFixture {
        users: vec![person()],
        profiles: vec![SeedProfile {
            user_id: person(),
            first_name: first.into(),
            last_name: last.into(),
            display_name: display.into(),
        }],
        ..SeedFixture::default()
    }
}

proptest! {
    #![proptest_config(proptest_config(128))]

    #[test]
    fn prop_a_seeded_profile_has_the_names_it_was_given_and_the_defaults_of_a_new_account(
        first in "\\PC{0,24}", last in "\\PC{0,24}", display in "\\PC{0,24}",
    ) {
        let w = world();
        block_on(seed(&w.app, &fixture(&first, &last, &display))).unwrap();
        let p = block_on(w.db.profile(person())).unwrap().expect("the user has a profile");
        prop_assert_eq!((&p.first_name, &p.last_name, &p.display_name), (&first, &last, &display));
        prop_assert_eq!((p.timezone.as_str(), p.is_onboarded, p.is_tour_completed), ("UTC", false, false));
        prop_assert_eq!(p.user_id, person());
    }

    #[test]
    fn prop_a_user_who_has_a_profile_keeps_its_row_and_takes_the_new_names(
        first in "\\PC{0,24}", then in "\\PC{0,24}", display in "\\PC{0,24}",
    ) {
        let w = world();
        block_on(seed(&w.app, &fixture(&first, "old", "old"))).unwrap();
        let before = block_on(w.db.profile(person())).unwrap().expect("a profile");
        block_on(seed(&w.app, &fixture(&then, "new", &display))).unwrap();
        let after = block_on(w.db.profile(person())).unwrap().expect("a profile");
        prop_assert_eq!(after.id, before.id, "the row is the same one");
        prop_assert_eq!((&after.first_name, &after.last_name, &after.display_name), (&then, &"new".to_string(), &display));
    }
}

#[test]
fn a_user_with_no_entry_in_the_profiles_is_given_no_profile() {
    let w = world();
    let f = SeedFixture { users: vec![person()], ..SeedFixture::default() };
    block_on(seed(&w.app, &f)).unwrap();
    assert!(block_on(w.db.profile(person())).unwrap().is_none());
}

/// `reset` reaches the database: the world's workspaces and users are gone, and the fixture that made them loads again.
#[test]
fn reset_empties_the_database_and_what_it_emptied_can_be_seeded_again() {
    let w = world();
    assert!(block_on(w.db.workspace_slug_taken("ws-a")).unwrap(), "the world has a workspace to lose");
    assert!(block_on(w.db.user_by_id(w.principal(Actor::AdminA).user_id)).unwrap().is_some());
    block_on(reset(&w.app)).unwrap();
    assert!(!block_on(w.db.workspace_slug_taken("ws-a")).unwrap());
    assert!(block_on(w.db.user_by_id(w.principal(Actor::AdminA).user_id)).unwrap().is_none());
    block_on(seed(&w.app, &fixture("Olive", "Owner", "oracle_owner"))).unwrap();
    assert!(block_on(w.db.profile(person())).unwrap().is_some());
}
