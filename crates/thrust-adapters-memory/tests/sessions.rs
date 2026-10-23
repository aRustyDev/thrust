//! The `sessions` directory table in the memory adapter (issue #36): a lookup returns a session exactly while its
//! row exists, is not revoked and has `expires_at > now`; a touch moves `last_seen_at` and nothing else; a revoke
//! sticks; an insert of a hash that exists leaves the row as it was. The Postgres adapter is held to the same answers
//! by the generated sequences in `thrust-adapters-selfhosted/tests/model`. Exhaustive over the finite domain.

use futures_executor::block_on;
use rstest::rstest;
use thrust_adapters_memory::MemoryDatabase;
use thrust_contracts::chrono::{DateTime, TimeDelta};
use thrust_contracts::*;
use thrust_ports::*;

fn now() -> Timestamp {
    DateTime::from_timestamp(1_760_000_000, 0).unwrap()
}

fn secs(n: i64) -> TimeDelta {
    TimeDelta::seconds(n)
}

fn hash(n: u8) -> TokenHash {
    TokenHash([n; 32])
}

const USER: UserId = UserId(Uuid::from_u128(7));

#[derive(Debug, Clone, Copy)]
enum Life {
    Live,
    /// `expires_at == now`: the row is live only while `expires_at > now`, so this is expired.
    ExpiresNow,
    Expired,
}

/// Session 1 exists, created 100 s ago and expiring as `life` says; session 2 never does.
fn with_session(life: Life) -> MemoryDatabase {
    let db = MemoryDatabase::new();
    // a session is live only for a user that exists and is active (issue #38), so the user is part of the setup
    let users = SeedFixture { users: vec![USER], ..SeedFixture::default() };
    block_on(db.seed(&SystemScope::mint(), &users)).unwrap();
    let expires_at = match life {
        Life::Live => now() + secs(60),
        Life::ExpiresNow => now(),
        Life::Expired => now() - secs(60),
    };
    let row = NewSession { token_hash: hash(1), user_id: USER, created_at: now() - secs(100), expires_at };
    block_on(db.insert_session(&row)).unwrap();
    db
}

#[rstest]
fn a_session_is_live_only_while_it_exists_unrevoked_and_unexpired(
    #[values(true, false)] known: bool,
    #[values(Life::Live, Life::ExpiresNow, Life::Expired)] life: Life,
    #[values(false, true)] revoked: bool,
) {
    let db = with_session(life);
    if revoked {
        block_on(db.revoke_session(&hash(1), now() - secs(5))).unwrap();
    }
    let got = block_on(db.session(&hash(if known { 1 } else { 2 }), now())).unwrap();
    let want = (known && matches!(life, Life::Live) && !revoked)
        .then_some(LiveSession { user_id: USER, last_seen_at: now() - secs(100) });
    assert_eq!(got, want, "known={known} {life:?} revoked={revoked}");
}

/// A touch moves `last_seen_at` and does not bring a revoked or an expired session back.
#[rstest]
fn a_touch_moves_last_seen_and_never_revives(
    #[values(Life::Live, Life::Expired)] life: Life,
    #[values(false, true)] revoked: bool,
) {
    let db = with_session(life);
    if revoked {
        block_on(db.revoke_session(&hash(1), now() - secs(5))).unwrap();
    }
    block_on(db.touch_session(&hash(1), now() - secs(1))).unwrap();
    block_on(db.touch_session(&hash(2), now())).unwrap(); // an unknown hash: nothing happens
    let got = block_on(db.session(&hash(1), now())).unwrap();
    let want = (matches!(life, Life::Live) && !revoked)
        .then_some(LiveSession { user_id: USER, last_seen_at: now() - secs(1) });
    assert_eq!(got, want, "{life:?} revoked={revoked}");
    assert_eq!(block_on(db.session(&hash(2), now())).unwrap(), None);
}

/// Revoking twice, or revoking what does not exist, is not an error and does not touch another session.
#[rstest]
fn a_revoke_sticks_and_reaches_only_the_session_it_names(#[values(1, 2)] twice_of: u8) {
    let db = with_session(Life::Live);
    let other = NewSession { token_hash: hash(3), user_id: USER, created_at: now(), expires_at: now() + secs(60) };
    block_on(db.insert_session(&other)).unwrap();
    block_on(db.revoke_session(&hash(twice_of), now())).unwrap();
    block_on(db.revoke_session(&hash(twice_of), now() + secs(1))).unwrap();
    let live = |n| block_on(db.session(&hash(n), now())).unwrap().is_some();
    assert_eq!((live(1), live(3)), (twice_of != 1, true));
}

/// Inserting a hash that exists keeps the first row: a re-seed must not extend or revive a session.
#[test]
fn an_insert_of_an_existing_hash_leaves_the_row_as_it_was() {
    let db = with_session(Life::Live);
    block_on(db.revoke_session(&hash(1), now() - secs(5))).unwrap();
    let again = NewSession {
        token_hash: hash(1),
        user_id: UserId(Uuid::from_u128(8)),
        created_at: now(),
        expires_at: now() + secs(9_999),
    };
    block_on(db.insert_session(&again)).unwrap();
    assert_eq!(block_on(db.session(&hash(1), now())).unwrap(), None, "the revoked row is still revoked");
}
