//! The identity tables in the SQLite adapter (issue #38 ported to the engine cells, migrations 0005 and 0007): users,
//! accounts, API keys and pending OIDC logins, and what they do to the `sessions` lookup. This is
//! `thrust-adapters-memory/tests/identity.rs` run against `SqliteDatabase` on rusqlite, case for case: exhaustive over the
//! finite domains (a key is known or not, live, expired or revoked, its user active or not), so the SQLite twin gives the
//! memory adapter's answers. The Postgres adapter is held to the same answers by the generated sequences in
//! `thrust-adapters-selfhosted/tests/model`. (A static case here is a pinned example the spec names, rule 9(b).)

use futures_executor::block_on;
use rstest::rstest;
use std::sync::Arc;

use thrust_adapters_sqlite::{NativeSql, SqliteDatabase};
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

const OWNER: UserId = UserId(Uuid::from_u128(7));
const OTHER: UserId = UserId(Uuid::from_u128(8));

fn fresh() -> SqliteDatabase {
    let db = SqliteDatabase::new(Arc::new(NativeSql::memory().unwrap()));
    db.migrate().unwrap();
    db
}

fn sys() -> SystemScope {
    SystemScope::mint()
}

fn user(id: UserId, email: Option<&str>, active: bool) -> UserRecord {
    UserRecord { id, email: email.map(str::to_string), password_hash: Some("$argon2id$stub".into()), is_active: active }
}

#[derive(Debug, Clone, Copy)]
enum Who {
    Active,
    Inactive,
    /// No `users` row at all.
    Absent,
}

fn with_user(who: Who) -> SqliteDatabase {
    let db = fresh();
    match who {
        Who::Active => block_on(db.seed_users(&sys(), &[user(OWNER, Some("o@example.test"), true)])).unwrap(),
        Who::Inactive => block_on(db.seed_users(&sys(), &[user(OWNER, Some("o@example.test"), false)])).unwrap(),
        Who::Absent => {}
    }
    db
}

// --- sessions and the user's state ----------------------------------------------------------------------------

/// A session is live only while its user exists and is active, in addition to the rules of issue #36.
#[rstest]
fn a_session_needs_an_existing_active_user(
    #[values(Who::Active, Who::Inactive, Who::Absent)] who: Who,
    #[values(false, true)] revoked: bool,
    #[values(false, true)] expired: bool,
) {
    let db = with_user(who);
    let expires_at = if expired { now() - secs(1) } else { now() + secs(60) };
    block_on(db.insert_session(&NewSession { token_hash: hash(1), user_id: OWNER, created_at: now() - secs(9), expires_at }))
        .unwrap();
    if revoked {
        block_on(db.revoke_session(&hash(1), now() - secs(5))).unwrap();
    }
    let got = block_on(db.session(&hash(1), now())).unwrap();
    let want = (matches!(who, Who::Active) && !revoked && !expired)
        .then_some(LiveSession { user_id: OWNER, last_seen_at: now() - secs(9) });
    assert_eq!(got, want, "{who:?} revoked={revoked} expired={expired}");
}

/// Reactivating a user brings back sessions that were neither revoked nor expired, and deactivating takes them away.
#[test]
fn deactivation_hides_a_session_and_reactivation_brings_it_back() {
    let db = with_user(Who::Active);
    block_on(db.insert_session(&NewSession {
        token_hash: hash(1),
        user_id: OWNER,
        created_at: now(),
        expires_at: now() + secs(60),
    }))
    .unwrap();
    let live = || block_on(db.session(&hash(1), now())).unwrap().is_some();
    assert!(live(), "active");
    assert!(block_on(db.set_user_active(&sys(), OWNER, false)).unwrap());
    assert!(!live(), "deactivated");
    assert!(block_on(db.set_user_active(&sys(), OWNER, true)).unwrap());
    assert!(live(), "reactivated");
    assert!(!block_on(db.set_user_active(&sys(), OTHER, false)).unwrap(), "an unknown user is not found");
}

/// Sign-out everywhere: every unrevoked session of the user, nobody else's, and a second call finds nothing left.
#[rstest]
fn revoke_user_sessions_reaches_every_session_of_that_user_and_no_other(
    #[values(0, 1, 3)] mine: u8,
    #[values(false, true)] one_already_revoked: bool,
) {
    let db = fresh();
    block_on(db.seed(&sys(), &SeedFixture { users: vec![OWNER, OTHER], ..SeedFixture::default() })).unwrap();
    let new = |n: u8, user| NewSession { token_hash: hash(n), user_id: user, created_at: now(), expires_at: now() + secs(60) };
    for n in 0..mine {
        block_on(db.insert_session(&new(n + 1, OWNER))).unwrap();
    }
    block_on(db.insert_session(&new(100, OTHER))).unwrap();
    let pre_revoked = u8::from(one_already_revoked && mine > 0);
    if pre_revoked == 1 {
        block_on(db.revoke_session(&hash(1), now())).unwrap();
    }
    let revoked = block_on(db.revoke_user_sessions(OWNER, now() + secs(1))).unwrap();
    assert_eq!(revoked, u64::from(mine - pre_revoked), "only the sessions that were still live are counted");
    assert_eq!(block_on(db.revoke_user_sessions(OWNER, now() + secs(2))).unwrap(), 0, "nothing left to revoke");
    for n in 0..mine {
        assert!(block_on(db.session(&hash(n + 1), now())).unwrap().is_none(), "session {n} survived");
    }
    assert!(block_on(db.session(&hash(100), now())).unwrap().is_some(), "another user's session was revoked");
}

// --- API keys -------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Life {
    NoExpiry,
    Live,
    /// `expires_at == now`: a key is live only while `expires_at > now`.
    ExpiresNow,
    Expired,
}

fn key(n: u8, user: UserId, life: Life) -> NewApiKey {
    NewApiKey {
        id: ApiKeyId(Uuid::from_u128(1000 + u128::from(n))),
        user_id: user,
        token_hash: hash(n),
        hash_version: 1,
        prefix: format!("thrk_{n:04}"),
        label: format!("key {n}"),
        created_at: now() - secs(100 - i64::from(n)),
        expires_at: match life {
            Life::NoExpiry => None,
            Life::Live => Some(now() + secs(60)),
            Life::ExpiresNow => Some(now()),
            Life::Expired => Some(now() - secs(60)),
        },
    }
}

#[rstest]
fn an_api_key_is_live_only_while_unrevoked_unexpired_and_its_user_active(
    #[values(true, false)] known: bool,
    #[values(Life::NoExpiry, Life::Live, Life::ExpiresNow, Life::Expired)] life: Life,
    #[values(false, true)] revoked: bool,
    #[values(Who::Active, Who::Inactive)] who: Who,
) {
    let db = with_user(who);
    let k = key(1, OWNER, life);
    block_on(db.insert_api_key(&k)).unwrap();
    if revoked {
        assert!(block_on(db.revoke_api_key(OWNER, k.id, now() - secs(5))).unwrap());
    }
    let got = block_on(db.api_key(&hash(if known { 1 } else { 2 }), now())).unwrap();
    let live = known && matches!(life, Life::NoExpiry | Life::Live) && !revoked && matches!(who, Who::Active);
    let want = live.then_some(LiveApiKey { id: k.id, user_id: OWNER, last_used_at: None, hash_version: 1 });
    assert_eq!(got, want, "known={known} {life:?} revoked={revoked} {who:?}");
}

/// Revoking is by owner and id: the owner's key is revoked (and stays so), another user's call finds nothing and
/// changes nothing, an unknown id finds nothing. The first revocation time is kept.
#[rstest]
fn revoke_api_key_is_scoped_to_the_owner_and_keeps_the_first_time(
    #[values(true, false)] by_owner: bool,
    #[values(true, false)] known_id: bool,
) {
    let db = fresh();
    block_on(db.seed(&sys(), &SeedFixture { users: vec![OWNER, OTHER], ..SeedFixture::default() })).unwrap();
    let k = key(1, OWNER, Life::NoExpiry);
    block_on(db.insert_api_key(&k)).unwrap();
    let id = if known_id { k.id } else { ApiKeyId(Uuid::from_u128(999)) };
    let caller = if by_owner { OWNER } else { OTHER };
    let found = block_on(db.revoke_api_key(caller, id, now())).unwrap();
    assert_eq!(found, by_owner && known_id);
    let listed = block_on(db.list_api_keys(OWNER)).unwrap();
    assert_eq!(listed[0].revoked_at, (by_owner && known_id).then_some(now()));
    // a second revoke is still "yours" and keeps the first time
    if by_owner && known_id {
        assert!(block_on(db.revoke_api_key(OWNER, k.id, now() + secs(30))).unwrap());
        assert_eq!(block_on(db.list_api_keys(OWNER)).unwrap()[0].revoked_at, Some(now()));
    }
    assert_eq!(block_on(db.api_key(&hash(1), now())).unwrap().is_some(), !(by_owner && known_id));
}

#[test]
fn a_touch_stamps_the_last_use_and_the_listing_shows_every_field_but_the_token_oldest_first() {
    let db = with_user(Who::Active);
    let (a, b) = (key(2, OWNER, Life::Live), key(1, OWNER, Life::NoExpiry));
    block_on(db.insert_api_key(&a)).unwrap();
    block_on(db.insert_api_key(&b)).unwrap();
    block_on(db.touch_api_key(a.id, now() + secs(3))).unwrap();
    block_on(db.touch_api_key(ApiKeyId(Uuid::from_u128(999)), now())).unwrap(); // unknown: nothing happens
    let listed = block_on(db.list_api_keys(OWNER)).unwrap();
    // `b` was created earlier (created_at is now - 100 + n)
    assert_eq!(listed.iter().map(|k| k.id).collect::<Vec<_>>(), [b.id, a.id]);
    assert_eq!(
        listed[1],
        ApiKeyInfo {
            id: a.id,
            prefix: a.prefix.clone(),
            label: a.label.clone(),
            created_at: a.created_at,
            expires_at: a.expires_at,
            revoked_at: None,
            last_used_at: Some(now() + secs(3)),
        }
    );
    assert_eq!(listed[0].last_used_at, None);
    assert!(block_on(db.list_api_keys(OTHER)).unwrap().is_empty(), "another user's keys are not listed");
    assert_eq!(block_on(db.api_key(&hash(2), now())).unwrap().unwrap().last_used_at, Some(now() + secs(3)));
}

#[rstest]
fn an_api_key_digest_or_id_that_exists_is_a_conflict_and_a_missing_owner_is_refused(
    #[values(true, false)] same_digest: bool,
    #[values(true, false)] same_id: bool,
) {
    let db = with_user(Who::Active);
    let first = key(1, OWNER, Life::NoExpiry);
    block_on(db.insert_api_key(&first)).unwrap();
    let mut second = key(2, OWNER, Life::NoExpiry);
    if same_digest {
        second.token_hash = first.token_hash;
    }
    if same_id {
        second.id = first.id;
    }
    let got = block_on(db.insert_api_key(&second));
    if same_digest || same_id {
        assert!(matches!(got, Err(DbError::Conflict(_))), "{got:?}");
    } else {
        got.unwrap();
    }
    // the key of a user that does not exist (the foreign key in Postgres)
    assert!(block_on(db.insert_api_key(&key(3, OTHER, Life::NoExpiry))).is_err());
}

/// Q-0052.3: a key carries the version of the scheme that made its digest. It must be positive (the column's CHECK), and
/// it comes back with the key.
#[rstest]
fn an_api_key_keeps_its_hash_version_and_a_version_that_is_not_positive_is_refused(
    #[values(i16::MIN, -1, 0, 1, 2, i16::MAX)] version: i16,
) {
    let db = with_user(Who::Active);
    let k = NewApiKey { hash_version: version, ..key(1, OWNER, Life::NoExpiry) };
    let got = block_on(db.insert_api_key(&k));
    if version > 0 {
        got.unwrap();
        assert_eq!(block_on(db.api_key(&k.token_hash, now())).unwrap().unwrap().hash_version, version);
    } else {
        assert!(matches!(got, Err(DbError::Backend(_))), "{got:?}");
        assert!(block_on(db.api_key(&k.token_hash, now())).unwrap().is_none(), "a refused key is not stored");
    }
}

/// `rehash_api_key` is a compare-and-set on the stored version: it changes a row only while the row is at `from`, and
/// then only the digest and the version. A row that is not at `from` (or an unknown id) answers `false` and is left
/// alone, and so is checked for nothing else (as `UPDATE ... WHERE hash_version = $2` touches no constraint for a row it
/// does not match); a matching row is refused for a version that is not positive (the CHECK) and for a digest that
/// another row holds (the UNIQUE index). Every combination, and the answer, the row and its neighbour afterwards.
#[rstest]
fn rehash_api_key_is_a_compare_and_set_on_the_stored_version(
    #[values(1, 2)] stored: i16,
    #[values(1, 2, 3)] from: i16,
    #[values(0, 1, 2, 3)] to: i16,
    #[values(true, false)] known_id: bool,
    #[values(true, false)] digest_taken: bool,
) {
    let db = with_user(Who::Active);
    let k = NewApiKey { hash_version: stored, ..key(1, OWNER, Life::Live) };
    let neighbour = key(2, OWNER, Life::NoExpiry);
    block_on(db.insert_api_key(&k)).unwrap();
    block_on(db.insert_api_key(&neighbour)).unwrap();
    block_on(db.touch_api_key(k.id, now() + secs(5))).unwrap();
    let new_hash = hash(100);
    if digest_taken {
        // a third row already holds the digest the move wants
        block_on(db.insert_api_key(&NewApiKey { token_hash: new_hash, ..key(3, OWNER, Life::NoExpiry) })).unwrap();
    }
    let before = block_on(db.list_api_keys(OWNER)).unwrap();
    let id = if known_id { k.id } else { ApiKeyId(Uuid::from_u128(999)) };

    let got = block_on(db.rehash_api_key(id, from, &new_hash, to));
    let label = format!("stored {stored} from {from} to {to} known={known_id} taken={digest_taken}");
    let matches_row = known_id && from == stored;
    let moved = match got {
        Ok(moved) => {
            assert!(matches_row || !moved, "{label}: only a matching row moves");
            moved
        }
        Err(e) => {
            assert!(matches_row, "{label}: a row that does not match is not looked at, so it cannot fail: {e}");
            match e {
                DbError::Backend(_) => assert!(to <= 0, "{label}"),
                DbError::Conflict(_) => assert!(to > 0 && digest_taken, "{label}"),
            }
            false
        }
    };
    assert_eq!(moved, matches_row && to > 0 && !digest_taken, "{label}");

    // what stays: every listed field of every key, the neighbour's digest, and either the old digest (unmoved) or the new
    assert_eq!(block_on(db.list_api_keys(OWNER)).unwrap(), before, "{label}");
    assert!(block_on(db.api_key(&neighbour.token_hash, now())).unwrap().is_some(), "{label}: the neighbour");
    let old = block_on(db.api_key(&k.token_hash, now())).unwrap();
    let new = block_on(db.api_key(&new_hash, now())).unwrap();
    if moved {
        assert!(old.is_none(), "{label}: the old digest no longer finds it");
        let new = new.expect("the new digest finds it");
        assert_eq!((new.id, new.hash_version, new.last_used_at), (k.id, to, Some(now() + secs(5))), "{label}");
    } else {
        let old = old.expect("the old digest still finds it");
        assert_eq!((old.id, old.hash_version), (k.id, stored), "{label}");
        // the digest is another row's if it is anyone's
        assert_eq!(new.is_some(), digest_taken, "{label}");
    }
}

// --- users, accounts ------------------------------------------------------------------------------------------

#[test]
fn seed_creates_users_by_id_and_does_not_overwrite_a_login() {
    let db = fresh();
    block_on(db.seed_users(&sys(), &[user(OWNER, Some("o@example.test"), true)])).unwrap();
    block_on(db.seed(&sys(), &SeedFixture { users: vec![OWNER, OTHER], ..SeedFixture::default() })).unwrap();
    // the id-only user exists and cannot be found by email; the user with a login is as it was
    assert_eq!(block_on(db.user_by_email("o@example.test")).unwrap().unwrap().id, OWNER);
    assert!(block_on(db.set_user_active(&sys(), OTHER, false)).unwrap(), "OTHER exists");
    let o = block_on(db.user_by_email("o@example.test")).unwrap().unwrap();
    assert_eq!(o.password_hash.as_deref(), Some("$argon2id$stub"));
    assert!(o.is_active);
}

#[rstest]
fn user_by_email_finds_the_user_active_or_not_by_the_exact_email(
    #[values(true, false)] active: bool,
    #[values("o@example.test", "O@example.test", "other@example.test", "")] asked: &str,
) {
    let db = fresh();
    block_on(db.seed_users(&sys(), &[user(OWNER, Some("o@example.test"), active)])).unwrap();
    let got = block_on(db.user_by_email(asked)).unwrap();
    // normalisation is the core's job: the adapter compares exactly
    assert_eq!(got.map(|u| (u.id, u.is_active)), (asked == "o@example.test").then_some((OWNER, active)));
}

#[test]
fn an_email_is_unique_and_seeding_the_same_user_again_updates_it() {
    let db = fresh();
    block_on(db.seed_users(&sys(), &[user(OWNER, Some("o@example.test"), true)])).unwrap();
    let clash = block_on(db.seed_users(&sys(), &[user(OTHER, Some("o@example.test"), true)]));
    assert!(matches!(clash, Err(DbError::Conflict(_))), "{clash:?}");
    // users without an email never clash
    block_on(db.seed_users(&sys(), &[user(OTHER, None, true), user(UserId(Uuid::from_u128(9)), None, true)])).unwrap();
    // the same user again: an upsert
    block_on(db.seed_users(&sys(), &[user(OWNER, Some("new@example.test"), false)])).unwrap();
    assert!(block_on(db.user_by_email("o@example.test")).unwrap().is_none());
    assert!(!block_on(db.user_by_email("new@example.test")).unwrap().unwrap().is_active);
}

fn account(user: UserId, sub: &str) -> NewAccount {
    NewAccount {
        provider: "https://idp.example.test".into(),
        provider_account_id: sub.into(),
        user_id: user,
        email: Some("o@example.test".into()),
        created_at: now(),
    }
}

#[rstest]
fn an_account_resolves_its_user_by_provider_and_subject_and_a_link_is_never_replaced(
    #[values("https://idp.example.test", "https://other.example.test")] provider: &str,
    #[values("sub-1", "sub-2")] sub: &str,
) {
    let db = fresh();
    block_on(db.seed_users(&sys(), &[user(OWNER, Some("o@example.test"), true), user(OTHER, Some("x@example.test"), false)]))
        .unwrap();
    block_on(db.link_account(&account(OWNER, "sub-1"))).unwrap();
    // linking the same subject to someone else leaves the first link
    block_on(db.link_account(&account(OTHER, "sub-1"))).unwrap();
    let got = block_on(db.user_for_account(provider, sub)).unwrap();
    let want = (provider == "https://idp.example.test" && sub == "sub-1").then_some(OWNER);
    assert_eq!(got.map(|u| u.id), want, "{provider} {sub}");
    // the link to an inactive user resolves too (the caller decides), and a link for no user is refused
    block_on(db.link_account(&account(OTHER, "sub-3"))).unwrap();
    assert!(!block_on(db.user_for_account("https://idp.example.test", "sub-3")).unwrap().unwrap().is_active);
    assert!(block_on(db.link_account(&account(UserId(Uuid::from_u128(77)), "sub-4"))).is_err());
}

// --- pending OIDC logins --------------------------------------------------------------------------------------

fn login(n: u8, ttl: i64, created: Timestamp) -> NewOidcLogin {
    NewOidcLogin {
        state_hash: hash(n),
        nonce: format!("nonce-{n}"),
        code_verifier: format!("verifier-{n}"),
        created_at: created,
        expires_at: created + secs(ttl),
    }
}

/// A pending login comes back once, with what was stored, and is gone for the next caller (a replayed callback).
#[rstest]
fn a_pending_login_is_taken_exactly_once(#[values(1, 2)] stored: u8, #[values(1, 2, 3)] asked: u8) {
    let db = fresh();
    block_on(db.insert_oidc_login(&login(stored, 600, now()))).unwrap();
    let first = block_on(db.take_oidc_login(&hash(asked))).unwrap();
    let second = block_on(db.take_oidc_login(&hash(asked))).unwrap();
    assert_eq!(second, None, "a second take must find nothing");
    let want = (asked == stored).then(|| OidcLogin {
        nonce: format!("nonce-{stored}"),
        code_verifier: format!("verifier-{stored}"),
        expires_at: now() + secs(600),
    });
    assert_eq!(first, want, "stored {stored}, asked {asked}");
    // taking one login leaves the others
    if asked != stored {
        assert!(block_on(db.take_oidc_login(&hash(stored))).unwrap().is_some());
    }
}

#[test]
fn an_insert_drops_the_logins_that_expired_before_it_and_refuses_a_state_that_exists() {
    let db = fresh();
    block_on(db.insert_oidc_login(&login(1, 60, now()))).unwrap();
    block_on(db.insert_oidc_login(&login(2, 600, now()))).unwrap();
    // 100 s later: login 1 has expired, login 2 has not
    block_on(db.insert_oidc_login(&login(3, 600, now() + secs(100)))).unwrap();
    assert!(block_on(db.take_oidc_login(&hash(1))).unwrap().is_none(), "the expired login was dropped");
    assert!(block_on(db.take_oidc_login(&hash(2))).unwrap().is_some());
    assert!(block_on(db.take_oidc_login(&hash(3))).unwrap().is_some());
    block_on(db.insert_oidc_login(&login(4, 600, now()))).unwrap();
    let dup = block_on(db.insert_oidc_login(&login(4, 600, now())));
    assert!(matches!(dup, Err(DbError::Conflict(_))), "{dup:?}");
}
