//! Generated sequences of `Database` and `Tx` port calls against the Postgres adapter and real Postgres (gap G-9 in
//! `docs/src/testing/status.md`).
//!
//! The test chooses every id itself, and the adapter's only own values (sequence and sort numbers) are predictable,
//! so the model is a pure planner: each generated operation becomes a script of port calls, each with the exact answer
//! the adapter must give, and the planner updates its state as if the adapter had agreed (it fails the case if not).
//! `model_postgres_adapter_matches_the_reference_model` runs the scripts against Postgres;
//! `model_postgres_strategy_reaches_every_step_class` runs the planner alone (the generator coverage control).
//!
//! The spec, written out (ADR-0043, ADR-0046):
//!
//! * `membership` resolves the workspace slug, the caller's membership of that workspace and of the project in one
//!   answer: `NoSuchProject` for a slug that names no workspace, a workspace the caller is not in, or a project
//!   outside the workspace the slug names; `NotAMember` for a workspace member without a project membership (a
//!   member row written with another workspace does not count); the member's role, with the workspace, otherwise;
//! * inside a transaction, `allocate` hands out the project's next sequence number and `sort_order` = 10 000 × that
//!   number, taking it in its own short transaction (ADR-0043 round 29, FR-22: the counter row is locked for one
//!   statement, not for the whole create): a number is taken for good, so a transaction that rolls back, is dropped
//!   unfinished or fails on a conflict leaves a GAP and the next transaction goes on after it; `allocate` under another workspace than the project's is refused, takes
//!   nothing, and the transaction is then rolled back;
//! * `insert_work_item` is a `Conflict` when the project already has a live row with the same external key, rows of
//!   the same transaction included; the transaction is then rolled back, as the core does;
//! * `soft_delete_work_item` is `true` exactly for a live row of that workspace and project (one inserted earlier in
//!   the same transaction included) and `false` for a deleted row, an unknown id or the wrong workspace; a soft
//!   delete frees the external key;
//! * nothing a transaction wrote (rows, deletes, outbox rows) is visible after a rollback or a drop, and all of it is
//!   after a commit; a dropped transaction neither holds locks nor poisons the pool (`mutant_discard_noop`);
//! * `list_work_items` returns exactly the project's rows in sequence order (live ones, or all with `WithDeleted`),
//!   and nothing through another workspace; `get_work_item` likewise for one id;
//! * `session` returns a session exactly while its row exists, is not revoked and has `expires_at > now` (a lookup at
//!   the expiry instant misses), with the user and the last time it was seen; `touch_session` moves `last_seen_at` and
//!   nothing else; `revoke_session` makes the session miss from then on (a second revoke changes nothing);
//!   `insert_session` of a hash that exists leaves the row as it was;
//! * a session is live only while its user is active as well (issue #38): `set_user_active` takes the user's sessions
//!   away and brings them back (a revoked or expired one stays gone), and `revoke_user_sessions` revokes exactly the
//!   user's sessions that were not revoked yet, returning how many, and nobody else's;
//! * `api_key` returns a key exactly while its row exists, is not revoked, has no expiry or `expires_at > now`, and its
//!   user is active, with the key's id, owner and last use; `touch_api_key` moves `last_used_at` and nothing else;
//!   `revoke_api_key` answers `true` for the owner's key (again on a second call, keeping the first time) and `false`
//!   for another user's key or an unknown id, which it does not touch; `list_api_keys` is the user's keys, oldest first;
//! * a key carries the version of the scheme that made its digest (Q-0052.3), which `api_key` returns; `rehash_api_key`
//!   is a compare-and-set on it: for a key that is at `from` it stores the new digest and version and answers `true`
//!   (from then on the old digest finds nothing, the new one finds the key, and nothing else about the key moved,
//!   revoked keys included); for a key that is not at `from`, or an unknown id, it answers `false` and changes nothing;
//!   a digest that another key holds is a `Conflict`, and the key stays as it was;
//! * `user_by_email` and `user_for_account` return the user, active or not, and nothing for an unknown email or a link
//!   that was never made; `link_account` of a subject that is linked already changes nothing;
//! * `insert_profile` creates a seeded user's profile (`true`) and refuses a second one for the same user (`false`,
//!   nothing changes); `update_profile` sets the fields its patch names, merges the patch's preferences key by key,
//!   replaces the onboarding steps whole, clears a field the patch says to clear, always sets `updated_at`, and answers the
//!   row as it is after (nothing at all for a user with no profile); `profile` reads it back;
//! * `insert_workspace` makes a workspace and its first member together or not at all (a taken slug is a `Conflict` that
//!   leaves no member row behind); `workspaces_of` lists a user's workspaces newest first, with their role and the count of
//!   active members, and only while the user is an active member; `workspace_access` resolves a slug for an active member and
//!   for nobody else; `workspace_slug_taken` is exact and case-sensitive; `set_workspace_member_active` hides a workspace from
//!   its member and shows it again; `workspace_members` lists everyone, the inactive too, with the person's profile fields;
//! * `project_roles` is the user's role in each project of the workspace whose member row names that workspace, and no other
//!   (a row that names another workspace than its project's is no role);
//! * `insert_user` creates a login that `user_by_email` then finds; a taken email or a taken id is a `Conflict` that
//!   changes nothing (the user that holds it is as it was, and the refused email names nobody);
//! * a pending OIDC login is returned by `take_oidc_login` once, and deleted by it (a second take, a replayed
//!   callback, finds nothing); an insert drops the logins whose `expires_at` is not after its `created_at`;
//! * `undispatched_outbox(n)` returns the first `n` undispatched rows by `(created_at, event_id)`, unchanged;
//!   `mark_outbox_dispatched` takes a row out of it (an unknown id is a no-op); `outbox_depth` counts what is left;
//! * `insert_activity` is `true` for a new `event_id` and `false` for one already projected, in any project;
//!   `list_activity` returns the project's rows in insertion order, optionally for one entity, and nothing through
//!   another workspace.
//!
//! Every case seeds its own workspaces and projects on one container. The outbox is global, so a case starts by
//! marking any rows an earlier (failed) case left behind as dispatched and ends by marking its own; timestamps come
//! from one clock for the whole run, so rows of different cases never interleave. Every port call has a 20 s timeout: a wedged pool fails, it does not
//! hang. Budget: `THRUST_MODEL_CASES` cases (default 32).

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, FailurePersistence, FileFailurePersistence, TestCaseError, TestRunner};
use thrust_contracts::chrono::{TimeDelta, TimeZone, Utc};
use thrust_contracts::*;
use thrust_ports::*;

use super::{env, new_id, Env};

const MAX_OPS: usize = 32;

// ---------------------------------------------------------------------------------------------
// Operations and their generators
// ---------------------------------------------------------------------------------------------

/// A seeded project: A in workspace A, B in workspace B.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum P {
    A,
    B,
}

/// The workspace a call names for a project: its own, or the other tenant's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ws {
    Own,
    Other,
}

/// Where a session lookup's `now` falls against the session's `expires_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum When {
    Before,
    /// Exactly at it: the session is already expired.
    AtExpiry,
    After,
}

/// The slug a `membership` call names for a project: its own workspace's, the other one's, or one that names none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sl {
    Own,
    Other,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
enum U {
    AdminA,
    MemberA,
    GuestA,
    /// In workspace A, member of nothing.
    OutsiderA,
    AdminB,
    /// In both workspaces, with a stray `project_members` row for project A that carries workspace B. It must not
    /// count.
    StrayB,
    /// In both workspaces, Admin of both projects: one user, two workspaces.
    Dual,
}

impl U {
    /// In declaration order, which is the order of `Fix::users`.
    const ALL: [U; 7] = [U::AdminA, U::MemberA, U::GuestA, U::OutsiderA, U::AdminB, U::StrayB, U::Dual];
}

#[derive(Debug, Clone, Copy)]
enum Step {
    /// `ext`: an external key from a small domain, so that keys collide.
    Create { proj: P, ext: Option<u8> },
    /// `pick`: one of the project's rows, committed or written earlier in this transaction (an unknown id if none).
    Delete { proj: P, ws: Ws, pick: usize },
    Outbox { proj: P },
    /// `allocate` naming the other tenant's workspace for the project. The core never makes this call (a `Scope`
    /// carries the project's own workspace); the adapter must still refuse it rather than start a second counter.
    AllocateElsewhere { proj: P },
}

#[derive(Debug, Clone, Copy)]
enum End {
    Commit,
    Rollback,
    /// Dropped without commit or rollback.
    Abandon,
}

#[derive(Debug, Clone, Copy)]
enum Ev {
    /// The event id of one of this case's outbox rows (a fresh id if there are none).
    Outbox(usize),
    /// An event id already projected into activity (a fresh id if there are none).
    Again(usize),
    Fresh,
}

/// What `Op::UserInsert` tries.
#[derive(Debug, Clone, Copy)]
enum UserIns {
    New,
    EmailTaken(usize),
    IdTaken(usize),
}

/// The shape of a `ProfilePatch` an `Op::ProfileUpdate` sends. The values inside are made fresh by the planner.
#[derive(Debug, Clone, Copy)]
enum PatchSel {
    Names,
    Cover { clear: bool },
    /// 0 sets `a` and `b`; 1 sets `b` and `c`: after both, `a` survives, `b` is the second one's, `c` is new.
    Prefs(u8),
    Onboarding,
    Flags,
    Workspace { clear: bool },
    Login,
    Empty,
}

#[derive(Debug, Clone)]
enum Op {
    Tx { steps: Vec<Step>, end: End },
    List { proj: P, ws: Ws, with_deleted: bool },
    /// `pick`: one of the project's rows, or `None` for an unknown id.
    Get { proj: P, ws: Ws, pick: Option<usize>, with_deleted: bool },
    Membership { proj: P, slug: Sl, user: U },
    /// `sess`: one of the case's two seeded sessions, or (2) a hash that was never inserted.
    SessionLookup { sess: usize, when: When },
    SessionTouch { sess: usize },
    SessionRevoke { sess: usize },
    /// Insert again a session that exists, with another user and expiry: the row must stay as it was.
    SessionReinsert { sess: usize },
    /// Deactivate or reactivate one of the three seeded login users (0..3).
    SetActive { who: usize, active: bool },
    /// Sign-out everywhere for one of the three seeded users.
    RevokeUserSessions { who: usize },
    /// `key`: one of the case's two inserted API keys, or (2) a digest that was never inserted.
    KeyLookup { key: usize, when: When },
    KeyTouch { key: usize },
    /// Look a key up by the digest of scheme `v` (1 to 3) instead of the one it is stored under: found only when `v` is
    /// the version it is stored at. Never expired (looked up just before its expiry).
    KeyLookupAs { key: usize, v: u8 },
    /// `rehash_api_key` of `key` to version `to` (1 to 3), compared against the version it is at (`stale_from: false`)
    /// or another one (`true`); `key` 2 is an id that was never inserted.
    KeyRehash { key: usize, stale_from: bool, to: u8 },
    /// Move key 0 or 1 to the digest the other one holds: a `Conflict`.
    KeyRehashCollide { key: usize },
    /// A key is moved to another version, looked up under the old digest (nothing) and the new one (found, new
    /// version), moved again from a version it is no longer at (refused), touched, and moved back.
    KeyMigration { key: usize, to: u8 },
    /// `by_owner: false`: another seeded user asks to revoke it.
    KeyRevoke { key: usize, by_owner: bool },
    KeyList { who: usize },
    /// `who`: a seeded login user, or (3) an email nobody has.
    EmailLookup { who: usize },
    /// `who`: a seeded user with a link, (2) one without, or (3) a subject nobody has.
    AccountLookup { who: usize },
    /// Link a subject that is linked already, to someone else: the first link stays.
    AccountRelink,
    /// `insert_user` of a new login, of one whose email a seeded user (0..3) holds, or of one whose id a seeded user holds.
    UserInsert { kind: UserIns },
    /// `insert_profile` for one of the three seeded users: the first time it is made, after that it is refused.
    ProfileInsert { who: usize },
    /// `update_profile` of one of the three seeded users, with a patch of this shape.
    ProfileUpdate { who: usize, sel: PatchSel },
    /// `profile` of one of the three seeded users.
    ProfileRead { who: usize },
    /// A compound op on brand-new users and workspaces (so the seeded workspaces, whose timestamps the database assigns, are not
    /// in the way): two workspaces for one new user, listed, looked up, deactivated and reactivated, and a creation that loses
    /// the race for a slug and must leave nothing behind.
    WorkspaceLife { with_profile: bool },
    /// `project_roles` of seeded user `user` in the workspace of project `proj`'s tenant.
    ProjectRolesRead { proj: P, user: U },
    /// Deactivate or reactivate seeded user `user`'s membership of project `ws`'s workspace (a no-op for a user who is not in
    /// it), then read back every query that depends on it.
    MemberActive { ws: P, user: U, active: bool },
    /// A compound op, for the sequences a single random op almost never reaches: the profile is made if there is none, then
    /// patched with both halves of the preferences (so the second must merge, not replace), the cover set and cleared, the
    /// onboarding steps replaced, the flags set, the last workspace set and cleared, the names changed, and read back.
    ProfileLife { who: usize },
    OidcInsert { ttl: u8 },
    /// Compound ops: the sequences a single random op almost never reaches, planned as the ops they are made of.
    /// A user (0 or 1) is deactivated, then everything that depends on it is looked up, then optionally reactivated.
    Deactivated { who: usize, restore: bool },
    /// A login is stored, taken, taken again (a replay) and a state that was never stored is taken.
    OidcRoundTrip { ttl: u8 },
    /// A login that expires (5 s) is followed, 5 ticks later, by an insert that sweeps it: taking it finds nothing, and
    /// taking the new one does.
    OidcExpiry,
    /// Key 0 or 1: looked up, touched twice (looked up in between), revoked by someone else (refused), looked up,
    /// revoked by its owner, looked up, revoked again, listed.
    KeyCycle { key: usize },
    /// `pick`: one of the logins ever inserted (taken, swept or still pending), or `None` for a state never stored.
    OidcTake { pick: Option<usize> },
    Undispatched { limit: u32 },
    /// `pick`: one of this case's outbox rows, dispatched or not, or `None` for an unknown event id.
    Mark { pick: Option<usize> },
    Depth,
    InsertActivity { proj: P, event: Ev },
    ListActivity { proj: P, ws: Ws, entity: Option<usize> },
}

fn proj() -> impl Strategy<Value = P> {
    prop_oneof![2 => Just(P::A), 1 => Just(P::B)]
}

fn ws() -> impl Strategy<Value = Ws> {
    prop_oneof![5 => Just(Ws::Own), 1 => Just(Ws::Other)]
}

fn when() -> impl Strategy<Value = When> {
    prop_oneof![Just(When::Before), Just(When::AtExpiry), Just(When::After)]
}

fn sl() -> impl Strategy<Value = Sl> {
    prop_oneof![3 => Just(Sl::Own), 1 => Just(Sl::Other), 1 => Just(Sl::Unknown)]
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (proj(), prop::option::weighted(0.5, 0..3u8)).prop_map(|(proj, ext)| Step::Create { proj, ext }),
        2 => (proj(), ws(), 0..16usize).prop_map(|(proj, ws, pick)| Step::Delete { proj, ws, pick }),
        2 => proj().prop_map(|proj| Step::Outbox { proj }),
        1 => proj().prop_map(|proj| Step::AllocateElsewhere { proj }),
    ]
}

fn end() -> impl Strategy<Value = End> {
    prop_oneof![5 => Just(End::Commit), 2 => Just(End::Rollback), 2 => Just(End::Abandon)]
}

fn user() -> impl Strategy<Value = U> {
    prop_oneof![
        Just(U::AdminA),
        Just(U::MemberA),
        Just(U::GuestA),
        Just(U::OutsiderA),
        Just(U::AdminB),
        Just(U::StrayB),
        Just(U::Dual)
    ]
}

fn ev() -> impl Strategy<Value = Ev> {
    prop_oneof![2 => (0..8usize).prop_map(Ev::Outbox), 1 => (0..8usize).prop_map(Ev::Again), 1 => Just(Ev::Fresh)]
}

fn patch_sel() -> impl Strategy<Value = PatchSel> {
    prop_oneof![
        Just(PatchSel::Names),
        any::<bool>().prop_map(|clear| PatchSel::Cover { clear }),
        (0..2u8).prop_map(PatchSel::Prefs),
        Just(PatchSel::Onboarding),
        Just(PatchSel::Flags),
        any::<bool>().prop_map(|clear| PatchSel::Workspace { clear }),
        Just(PatchSel::Login),
        Just(PatchSel::Empty),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (prop::collection::vec(step(), 1..5), end()).prop_map(|(steps, end)| Op::Tx { steps, end }),
        2 => (proj(), ws(), any::<bool>()).prop_map(|(proj, ws, with_deleted)| Op::List { proj, ws, with_deleted }),
        2 => (proj(), ws(), prop::option::weighted(0.85, 0..16usize), any::<bool>())
            .prop_map(|(proj, ws, pick, with_deleted)| Op::Get { proj, ws, pick, with_deleted }),
        2 => (proj(), sl(), user()).prop_map(|(proj, slug, user)| Op::Membership { proj, slug, user }),
        3 => (0..3usize, when()).prop_map(|(sess, when)| Op::SessionLookup { sess, when }),
        1 => (0..3usize).prop_map(|sess| Op::SessionTouch { sess }),
        1 => (0..3usize).prop_map(|sess| Op::SessionRevoke { sess }),
        1 => (0..2usize).prop_map(|sess| Op::SessionReinsert { sess }),
        3 => (0..3usize, any::<bool>()).prop_map(|(who, active)| Op::SetActive { who, active }),
        1 => (0..3usize).prop_map(|who| Op::RevokeUserSessions { who }),
        2 => (0..3usize, when()).prop_map(|(key, when)| Op::KeyLookup { key, when }),
        1 => (0..3usize).prop_map(|key| Op::KeyTouch { key }),
        2 => (0..3usize, 1..=3u8).prop_map(|(key, v)| Op::KeyLookupAs { key, v }),
        3 => (0..3usize, any::<bool>(), 1..=3u8).prop_map(|(key, stale_from, to)| Op::KeyRehash { key, stale_from, to }),
        1 => (0..2usize).prop_map(|key| Op::KeyRehashCollide { key }),
        2 => (0..2usize, 2..=3u8).prop_map(|(key, to)| Op::KeyMigration { key, to }),
        1 => (0..3usize, prop::bool::weighted(0.75)).prop_map(|(key, by_owner)| Op::KeyRevoke { key, by_owner }),
        1 => (0..3usize).prop_map(|who| Op::KeyList { who }),
        1 => (0..4usize).prop_map(|who| Op::EmailLookup { who }),
        1 => (0..4usize).prop_map(|who| Op::AccountLookup { who }),
        1 => Just(Op::AccountRelink),
        2 => prop_oneof![
            2 => Just(UserIns::New),
            1 => (0..3usize).prop_map(UserIns::EmailTaken),
            1 => (0..3usize).prop_map(UserIns::IdTaken),
        ]
        .prop_map(|kind| Op::UserInsert { kind }),
        1 => (0..3usize).prop_map(|who| Op::ProfileInsert { who }),
        3 => ((0..3usize), patch_sel()).prop_map(|(who, sel)| Op::ProfileUpdate { who, sel }),
        1 => (0..3usize).prop_map(|who| Op::ProfileRead { who }),
        2 => (0..3usize).prop_map(|who| Op::ProfileLife { who }),
        2 => any::<bool>().prop_map(|with_profile| Op::WorkspaceLife { with_profile }),
        2 => (proj(), user()).prop_map(|(proj, user)| Op::ProjectRolesRead { proj, user }),
        3 => (proj(), user(), any::<bool>()).prop_map(|(ws, user, active)| Op::MemberActive { ws, user, active }),
        2 => (0..2usize, any::<bool>()).prop_map(|(who, restore)| Op::Deactivated { who, restore }),
        2 => prop_oneof![Just(5u8), Just(100u8)].prop_map(|ttl| Op::OidcRoundTrip { ttl }),
        2 => (0..2usize).prop_map(|key| Op::KeyCycle { key }),
        2 => Just(Op::OidcExpiry),
        3 => prop_oneof![Just(5u8), Just(100u8)].prop_map(|ttl| Op::OidcInsert { ttl }),
        4 => prop::option::weighted(0.85, 0..8usize).prop_map(|pick| Op::OidcTake { pick }),
        1 => (1..6u32).prop_map(|limit| Op::Undispatched { limit }),
        1 => prop::option::weighted(0.85, 0..16usize).prop_map(|pick| Op::Mark { pick }),
        1 => Just(Op::Depth),
        2 => (proj(), ev()).prop_map(|(proj, event)| Op::InsertActivity { proj, event }),
        1 => (proj(), ws(), prop::option::weighted(0.5, 0..16usize))
            .prop_map(|(proj, ws, entity)| Op::ListActivity { proj, ws, entity }),
    ]
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(op(), 1..=MAX_OPS)
}

// ---------------------------------------------------------------------------------------------
// The planner
// ---------------------------------------------------------------------------------------------

/// What kind of step the planner produced. The generator coverage control requires every one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Created,
    /// A create reusing the external key of a soft-deleted row.
    ExternalKeyFreed,
    Conflict,
    /// A conflict with a row written earlier in the same transaction.
    ConflictWithinTx,
    DeletedCommitted,
    /// A delete of a row inserted earlier in the same transaction.
    DeletedWithinTx,
    DeleteAlreadyDeleted,
    DeleteWrongWorkspace,
    DeleteUnknown,
    OutboxWritten,
    Committed,
    RolledBack,
    Abandoned,
    /// A transaction rolled back because of a conflict.
    FailedAndRolledBack,
    /// A transaction that took sequence numbers and did not commit: the numbers stay taken (FR-22), the next one skips them.
    NumbersKeptAfterAbort,
    ListedVisible,
    ListedWithDeleted,
    ListedWrongWorkspace,
    GotLive,
    /// A visible get of a deleted row: `None`.
    GotDeletedHidden,
    GotWithDeleted,
    GotWrongWorkspace,
    GotUnknown,
    IsMember,
    NotAMember,
    /// A project outside the workspace the slug names.
    NoSuchProject,
    /// A slug that names no workspace.
    UnknownSlug,
    /// The project's own slug, by a user who is not in that workspace.
    NotInWorkspace,
    SessionLive,
    /// A hash that was never inserted.
    SessionUnknown,
    /// A revoked session, which must not be found.
    SessionRevokedHidden,
    /// An unrevoked session looked up at or after its `expires_at`, which must not be found.
    SessionExpiredHidden,
    SessionTouched,
    SessionRevoked,
    /// A revoke of a session that is already revoked.
    SessionRevokedAgain,
    SessionReinsertIgnored,
    /// An unrevoked, unexpired session whose user is deactivated, which must not be found.
    SessionUserInactiveHidden,
    UserDeactivated,
    UserReactivated,
    /// Sign-out everywhere that revoked at least one session.
    UserSessionsRevoked,
    /// Sign-out everywhere with nothing left to revoke.
    UserSessionsRevokedNone,
    KeyLive,
    KeyUnknown,
    KeyRevokedHidden,
    KeyExpiredHidden,
    KeyUserInactiveHidden,
    KeyTouched,
    /// A lookup by the digest of a version the key is not stored at: nothing.
    KeyOtherVersionHidden,
    /// `rehash_api_key` of a key at the version it names.
    KeyRehashed,
    /// The same for a key that is revoked: it moves, and stays revoked.
    KeyRehashedWhileRevoked,
    /// `rehash_api_key` naming a version the key is not at: `false`, nothing changes.
    KeyRehashStale,
    /// `rehash_api_key` of an id that was never inserted: `false`.
    KeyRehashUnknown,
    /// `rehash_api_key` to a digest another key holds: a `Conflict`, nothing changes.
    KeyRehashConflict,
    KeyRevoked,
    /// The owner revokes a key that is revoked already: still `true`, first time kept.
    KeyRevokedAgain,
    /// Another user's revoke: `false`, nothing changes.
    KeyRevokeRefused,
    KeyListed,
    EmailFound,
    EmailFoundInactive,
    EmailUnknown,
    AccountFound,
    AccountFoundInactive,
    AccountUnlinked,
    AccountRelinkKept,
    /// `insert_user` of a new login: found by email afterwards.
    UserInserted,
    /// `insert_user` with an email a user holds: a `Conflict`, the holder unchanged.
    UserInsertEmailTaken,
    /// `insert_user` with an id a user holds: a `Conflict`, the new email names nobody.
    UserInsertIdTaken,
    /// `insert_profile` for a user with none: made, and read back.
    ProfileInserted,
    /// `insert_profile` for a user that has one: `false`, the profile unchanged.
    ProfileInsertKept,
    /// `update_profile` of a profile: the patched row, read back.
    ProfileUpdated,
    /// `update_profile` of a user with no profile: nothing, and nothing is made.
    ProfileUpdateMissing,
    /// `update_profile` whose preferences leave a key the profile already holds unnamed: the key must survive (a merge, not
    /// a replace).
    ProfilePrefsMerged,
    /// `insert_workspace`: a workspace and its first member, read back through every query that should see them.
    WorkspaceMade,
    /// `insert_workspace` with a taken slug: a `Conflict`, and the loser has no workspace and no membership.
    WorkspaceSlugConflict,
    /// `set_workspace_member_active(false)`: the workspace leaves the member's list and their slug lookup, stays in the member
    /// list as suspended, and comes back when reactivated.
    WorkspaceMemberDeactivated,
    /// `set_workspace_member_active(true)` on a membership that was deactivated.
    WorkspaceMemberReactivated,
    /// A workspace's member list shows the profile fields of a member who has a profile.
    WorkspaceMemberProfileShown,
    /// `project_roles` with at least one role.
    ProjectRolesSome,
    /// `project_roles` with none: a user who is in no project of the workspace, or whose only row names the wrong workspace.
    ProjectRolesNone,
    OidcInserted,
    /// An insert that dropped an expired login.
    OidcSwept,
    OidcTaken,
    /// A take of a login that was taken already: nothing.
    OidcReplayHidden,
    /// A take of a login that an insert swept away: nothing.
    OidcSweptHidden,
    OidcUnknown,
    UndispatchedSome,
    Marked,
    MarkedAgain,
    MarkedUnknown,
    Depth,
    ActivityInserted,
    ActivityRedelivered,
    ActivityListed,
    ActivityListedForEntity,
    ActivityWrongWorkspace,
    /// `allocate` under the other tenant's workspace, refused; the transaction is rolled back.
    AllocateWrongWorkspace,
    /// `membership` of the stray member row: `NotAMember`.
    StrayMemberIgnored,
}

impl Class {
    const ALL: [Class; 97] = [
        Class::Created,
        Class::ExternalKeyFreed,
        Class::Conflict,
        Class::ConflictWithinTx,
        Class::DeletedCommitted,
        Class::DeletedWithinTx,
        Class::DeleteAlreadyDeleted,
        Class::DeleteWrongWorkspace,
        Class::DeleteUnknown,
        Class::OutboxWritten,
        Class::Committed,
        Class::RolledBack,
        Class::Abandoned,
        Class::FailedAndRolledBack,
        Class::NumbersKeptAfterAbort,
        Class::ListedVisible,
        Class::ListedWithDeleted,
        Class::ListedWrongWorkspace,
        Class::GotLive,
        Class::GotDeletedHidden,
        Class::GotWithDeleted,
        Class::GotWrongWorkspace,
        Class::GotUnknown,
        Class::IsMember,
        Class::NotAMember,
        Class::NoSuchProject,
        Class::UnknownSlug,
        Class::NotInWorkspace,
        Class::SessionLive,
        Class::SessionUnknown,
        Class::SessionRevokedHidden,
        Class::SessionExpiredHidden,
        Class::SessionTouched,
        Class::SessionRevoked,
        Class::SessionRevokedAgain,
        Class::SessionReinsertIgnored,
        Class::SessionUserInactiveHidden,
        Class::UserDeactivated,
        Class::UserReactivated,
        Class::UserSessionsRevoked,
        Class::UserSessionsRevokedNone,
        Class::KeyLive,
        Class::KeyUnknown,
        Class::KeyRevokedHidden,
        Class::KeyExpiredHidden,
        Class::KeyUserInactiveHidden,
        Class::KeyTouched,
        Class::KeyOtherVersionHidden,
        Class::KeyRehashed,
        Class::KeyRehashedWhileRevoked,
        Class::KeyRehashStale,
        Class::KeyRehashUnknown,
        Class::KeyRehashConflict,
        Class::KeyRevoked,
        Class::KeyRevokedAgain,
        Class::KeyRevokeRefused,
        Class::KeyListed,
        Class::EmailFound,
        Class::EmailFoundInactive,
        Class::EmailUnknown,
        Class::AccountFound,
        Class::AccountFoundInactive,
        Class::AccountUnlinked,
        Class::AccountRelinkKept,
        Class::UserInserted,
        Class::UserInsertEmailTaken,
        Class::UserInsertIdTaken,
        Class::ProfileInserted,
        Class::ProfileInsertKept,
        Class::ProfileUpdated,
        Class::ProfileUpdateMissing,
        Class::ProfilePrefsMerged,
        Class::WorkspaceMade,
        Class::WorkspaceSlugConflict,
        Class::WorkspaceMemberDeactivated,
        Class::WorkspaceMemberReactivated,
        Class::WorkspaceMemberProfileShown,
        Class::ProjectRolesSome,
        Class::ProjectRolesNone,
        Class::OidcInserted,
        Class::OidcSwept,
        Class::OidcTaken,
        Class::OidcReplayHidden,
        Class::OidcSweptHidden,
        Class::OidcUnknown,
        Class::UndispatchedSome,
        Class::Marked,
        Class::MarkedAgain,
        Class::MarkedUnknown,
        Class::Depth,
        Class::ActivityInserted,
        Class::ActivityRedelivered,
        Class::ActivityListed,
        Class::ActivityListedForEntity,
        Class::ActivityWrongWorkspace,
        Class::AllocateWrongWorkspace,
        Class::StrayMemberIgnored,
    ];
}

/// One port call and the answer the adapter must give.
#[derive(Debug, Clone)]
enum Call {
    Begin { ws: WorkspaceId, p: ProjectId },
    /// `want: None`: the adapter must refuse the allocation.
    Allocate { ws: WorkspaceId, p: ProjectId, want: Option<Allocation> },
    Insert { item: WorkItem, conflict: bool },
    SoftDelete { ws: WorkspaceId, p: ProjectId, id: WorkItemId, at: Timestamp, want: bool },
    InsertOutbox { row: OutboxRow },
    Commit,
    Rollback,
    Abandon,
    List { ws: WorkspaceId, p: ProjectId, vis: Visibility, want: Vec<WorkItem> },
    Get { ws: WorkspaceId, p: ProjectId, id: WorkItemId, vis: Visibility, want: Option<WorkItem> },
    Membership { slug: WorkspaceSlug, p: ProjectId, user: UserId, want: Membership },
    InsertSession { s: NewSession },
    Session { hash: TokenHash, now: Timestamp, want: Option<LiveSession> },
    TouchSession { hash: TokenHash, at: Timestamp },
    RevokeSession { hash: TokenHash, at: Timestamp },
    SeedUsers { users: Vec<UserRecord> },
    /// `conflict`: the adapter must answer `DbError::Conflict`.
    InsertUser { u: UserRecord, conflict: bool },
    /// `want`: whether the profile was made (`false`: the user has one already).
    InsertProfile { p: UserProfile, want: bool },
    Profile { user: UserId, want: Option<UserProfile> },
    UpdateProfile { user: UserId, patch: ProfilePatch, at: Timestamp, want: Option<UserProfile> },
    /// `conflict`: the adapter must answer `DbError::Conflict`.
    InsertWorkspace { w: NewWorkspace, conflict: bool },
    SlugTaken { slug: String, want: bool },
    AnyWorkspace { want: bool },
    WorkspacesOf { user: UserId, want: Vec<WorkspaceListing> },
    WorkspaceAccessOf { slug: WorkspaceSlug, user: UserId, want: Option<WorkspaceAccess> },
    /// `want`: whether the membership exists.
    SetMemberActive { ws: WorkspaceId, user: UserId, active: bool, want: bool },
    /// The three calls below take a `WorkspaceScope`: `(workspace, user, role)` are what the executor mints it from.
    WorkspaceMemberOf { scope: (WorkspaceId, UserId, Role), want: Option<WorkspaceMember> },
    WorkspaceMembersOf { scope: (WorkspaceId, UserId, Role), want: Vec<MemberWithUser> },
    ProjectRolesOf { scope: (WorkspaceId, UserId, Role), want: Vec<(ProjectId, Role)> },
    /// `workspaces_of(user)` for a seeded user, as `(workspace, role, active members)` sorted by workspace id: the seed's
    /// timestamps are the database's own, so the listing's order and its timestamps are not compared here.
    SeededListing { user: UserId, want: Vec<(WorkspaceId, Role, u64)> },
    /// `workspace_members` of a seeded workspace, as `(user, is_active)` sorted by user id, for the same reason.
    SeededMembers { ws: WorkspaceId, want: Vec<(UserId, bool)> },
    LinkAccount { a: NewAccount },
    InsertApiKey { k: NewApiKey },
    SetActive { user: UserId, active: bool },
    RevokeUserSessions { user: UserId, at: Timestamp, want: u64 },
    ApiKey { hash: TokenHash, now: Timestamp, want: Option<LiveApiKey> },
    TouchApiKey { id: ApiKeyId, at: Timestamp },
    RehashApiKey { id: ApiKeyId, from: i16, new_hash: TokenHash, to: i16, want: Rehash },
    RevokeApiKey { user: UserId, id: ApiKeyId, at: Timestamp, want: bool },
    ListApiKeys { user: UserId, want: Vec<ApiKeyInfo> },
    UserByEmail { email: String, want: Option<UserRecord> },
    UserForAccount { provider: String, sub: String, want: Option<UserRecord> },
    InsertOidcLogin { l: NewOidcLogin },
    TakeOidcLogin { state: TokenHash, want: Option<OidcLogin> },
    Undispatched { limit: u32, want: Vec<OutboxRow> },
    Mark { event: EventId, at: Timestamp },
    Depth { want: u64 },
    InsertActivity { a: Activity, want: bool },
    ListActivity { ws: WorkspaceId, p: ProjectId, entity: Option<WorkItemId>, want: Vec<Activity> },
}

/// What `rehash_api_key` must answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rehash {
    /// `Ok`, with whether a row changed.
    Changed(bool),
    /// `Err(DbError::Conflict)`.
    Conflict,
}

/// The ids of one case's fixture, chosen by the test.
#[derive(Debug, Clone)]
struct Fix {
    ws: [WorkspaceId; 2],
    /// Slugs are unique in the database and one container serves every case.
    slug: [WorkspaceSlug; 2],
    p: [ProjectId; 2],
    /// AdminA, MemberA, GuestA, OutsiderA, AdminB, StrayB, Dual.
    users: [UserId; 7],
    /// The hashes of two seeded sessions and of one that is never inserted.
    tokens: [TokenHash; 3],
    /// Two API keys that are inserted, and a digest that never is. The digest is the key's version 1 digest.
    keys: [(ApiKeyId, TokenHash); 3],
    /// The same three keys' digests under versions 2 and 3, for `rehash_api_key` to move them to.
    moved: [[TokenHash; 2]; 3],
    /// Unique per case: the unique index on `users.email` spans the whole container.
    emails: [String; 3],
    /// The issuer of the case's account links: unique per case, `(provider, sub)` is global.
    provider: String,
}

fn token_hash() -> TokenHash {
    let mut b = [0u8; 32];
    b[..16].copy_from_slice(new_id().as_bytes());
    b[16..].copy_from_slice(new_id().as_bytes());
    TokenHash(b)
}

impl Fix {
    fn new() -> Fix {
        Fix {
            ws: [WorkspaceId(new_id()), WorkspaceId(new_id())],
            slug: [
                WorkspaceSlug::new(format!("ws-a-{}", new_id().simple())),
                WorkspaceSlug::new(format!("ws-b-{}", new_id().simple())),
            ],
            p: [ProjectId(new_id()), ProjectId(new_id())],
            users: std::array::from_fn(|_| UserId(new_id())),
            tokens: std::array::from_fn(|_| token_hash()),
            keys: std::array::from_fn(|_| (ApiKeyId(new_id()), token_hash())),
            moved: std::array::from_fn(|_| std::array::from_fn(|_| token_hash())),
            emails: std::array::from_fn(|_| format!("u-{}@example.test", new_id().simple())),
            provider: format!("https://idp-{}.example.test", new_id().simple()),
        }
    }

    fn seed(&self) -> SeedFixture {
        let [admin_a, member_a, guest_a, outsider_a, admin_b, stray_b, dual] = self.users;
        let m = |s: usize, user_id, role| SeedMembership { project_id: self.p[s], workspace_id: self.ws[s], user_id, role };
        let in_ws = |s: usize, user_id| SeedWorkspaceMember { workspace_id: self.ws[s], user_id };
        SeedFixture {
            workspaces: (0..2).map(|s| SeedWorkspace { id: self.ws[s], slug: self.slug[s].clone() }).collect(),
            users: self.users.to_vec(),
            workspace_members: vec![
                in_ws(0, admin_a),
                in_ws(0, member_a),
                in_ws(0, guest_a),
                in_ws(0, outsider_a),
                in_ws(1, admin_b),
                in_ws(0, stray_b),
                in_ws(1, stray_b),
                in_ws(0, dual),
                in_ws(1, dual),
            ],
            projects: vec![
                SeedProject { id: self.p[0], workspace_id: self.ws[0], name: "A".into() },
                SeedProject { id: self.p[1], workspace_id: self.ws[1], name: "B".into() },
            ],
            memberships: vec![
                m(0, admin_a, Role::Admin),
                m(0, member_a, Role::Member),
                m(0, guest_a, Role::Guest),
                m(1, admin_b, Role::Admin),
                m(0, dual, Role::Admin),
                m(1, dual, Role::Admin),
                // project A's row, written with workspace B
                SeedMembership { project_id: self.p[0], workspace_id: self.ws[1], user_id: stray_b, role: Role::Admin },
            ],
            sessions: vec![],
            ..SeedFixture::default()
        }
    }

    fn user(&self, u: U) -> UserId {
        self.users[u as usize]
    }

    /// Key `key`'s digest under scheme `version` (1 to 3): what the planner stores it as after a move to that version.
    fn digest(&self, key: usize, version: i16) -> TokenHash {
        match version {
            1 => self.keys[key].1,
            v => self.moved[key][usize::from(v.unsigned_abs()) - 2],
        }
    }

    /// The three users who have a login: AdminA, MemberA, GuestA. Active, with a stub hash (the adapter never looks
    /// inside it).
    fn login_users(&self) -> Vec<UserRecord> {
        (0..3).map(|i| self.login_user(i, true)).collect()
    }

    fn login_user(&self, i: usize, is_active: bool) -> UserRecord {
        UserRecord {
            id: self.users[i],
            email: Some(self.emails[i].clone()),
            password_hash: Some(format!("$argon2id$stub{i}")),
            is_active,
        }
    }

    /// Users 0 and 1 are linked to `(provider, sub-i)`; user 2 is not.
    fn account(&self, i: usize, user: usize) -> NewAccount {
        NewAccount {
            provider: self.provider.clone(),
            provider_account_id: format!("sub-{i}"),
            user_id: self.users[user],
            email: Some(self.emails[user].clone()),
            created_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
        }
    }

    /// The workspace a call names for `proj`.
    fn ws_for(&self, proj: P, ws: Ws) -> WorkspaceId {
        let own = slot(proj);
        self.ws[if ws == Ws::Own { own } else { 1 - own }]
    }

    /// The slug a `membership` call names for `proj`.
    fn slug_for(&self, proj: P, sl: Sl) -> WorkspaceSlug {
        let own = slot(proj);
        match sl {
            Sl::Own => self.slug[own].clone(),
            Sl::Other => self.slug[1 - own].clone(),
            Sl::Unknown => WorkspaceSlug::new("no-such-workspace"),
        }
    }
}

fn slot(p: P) -> usize {
    match p {
        P::A => 0,
        P::B => 1,
    }
}

/// Which workspaces (A, B) a user belongs to, written out from `Fix::seed`.
fn in_workspace(user: U) -> [bool; 2] {
    match user {
        U::AdminA | U::MemberA | U::GuestA | U::OutsiderA => [true, false],
        U::AdminB => [false, true],
        U::StrayB | U::Dual => [true, true],
    }
}

/// `v` ordered by the id `key` names: the seeded rows' timestamps are the database's, so those answers are compared as sets.
fn sorted_by_id<T>(mut v: Vec<T>, key: impl Fn(&T) -> Uuid) -> Vec<T> {
    v.sort_by_key(|t| key(t));
    v
}

/// A workspace and its first member for a brand-new `owner`, stamped now (so a later one is newer), with every field set to
/// something the read-back can tell from a default.
fn new_workspace(owner: UserId, role: Role, tag: &str) -> NewWorkspace {
    let now = tick();
    let id = WorkspaceId(new_id());
    NewWorkspace {
        workspace: Workspace {
            id,
            slug: WorkspaceSlug::new(format!("{tag}-{}", new_id().simple())),
            name: format!("Workspace {tag}"),
            owner_id: Some(owner),
            organization_size: Some("11-50".into()),
            timezone: "Europe/Berlin".into(),
            background_color: "#112233".into(),
            created_at: now,
            updated_at: now,
        },
        owner: WorkspaceMember {
            id: new_id(),
            workspace_id: id,
            user_id: owner,
            role,
            company_role: Some("Engineer".into()),
            is_active: true,
            props: serde_json::json!({"filters": {"state": ["a"]}, "n": 3}),
            created_at: now,
            updated_at: now,
        },
    }
}

/// A user's role in a project, written out from `Fix::seed`. `StrayB`'s row for project A names workspace B, so it is
/// not a membership of project A.
fn role_in(proj: P, user: U) -> Option<Role> {
    match (proj, user) {
        (P::A, U::AdminA) | (P::B, U::AdminB) | (_, U::Dual) => Some(Role::Admin),
        (P::A, U::MemberA) => Some(Role::Member),
        (P::A, U::GuestA) => Some(Role::Guest),
        _ => None,
    }
}

/// The spec's membership answer: only the project's own slug resolves it, and only for an ACTIVE member of that workspace
/// (`active`: the user's membership of it, which a user who is not a member has too).
fn membership(fix: &Fix, proj: P, sl: Sl, user: U, active: bool) -> Membership {
    let s = slot(proj);
    if sl != Sl::Own || !in_workspace(user)[s] || !active {
        return Membership::NoSuchProject;
    }
    match role_in(proj, user) {
        Some(role) => Membership::Member { workspace_id: fix.ws[s], role },
        None => Membership::NotAMember,
    }
}

#[derive(Debug, Clone)]
struct KeyModel {
    new: NewApiKey,
    revoked: Option<Timestamp>,
    last_used: Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoginState {
    Pending,
    Taken,
    Swept,
}

/// A seeded session as the model knows it.
#[derive(Debug, Clone)]
struct SessModel {
    new: NewSession,
    revoked: bool,
    last_seen: Timestamp,
}

#[derive(Debug)]
struct Model {
    fix: Fix,
    /// Sessions 0 (expires far off) and 1 (expires 100 s after it was created); session 2 does not exist.
    sessions: Vec<SessModel>,
    /// Whether each of the three seeded login users (the first three of `Fix::users`) is active.
    active: [bool; 3],
    /// Keys 0 (no expiry) and 1 (expires 100 s after it was created, for user 1); key 2 does not exist.
    api_keys: Vec<KeyModel>,
    /// Every OIDC login ever inserted, with whether it is still pending (not taken, not swept).
    logins: Vec<(NewOidcLogin, LoginState)>,
    /// The profile of each of the three seeded login users, once an op has made it.
    profiles: [Option<UserProfile>; 3],
    /// Whether each seeded user's membership of each seeded workspace is active (it is for users who are not members).
    member_active: [[bool; 2]; 7],
    /// Committed rows per project, in sequence order.
    rows: [Vec<WorkItem>; 2],
    counter: [i64; 2],
    /// This case's committed outbox rows, and whether they are dispatched.
    outbox: Vec<(OutboxRow, bool)>,
    /// Activity rows inserted by this case, in insertion order.
    activity: Vec<Activity>,
    /// Every event id projected so far (the uniqueness is global).
    projected: BTreeSet<EventId>,
}

/// One clock for the whole run: strictly increasing, whole seconds (so Postgres stores them exactly), and never
/// repeated across cases.
static CLOCK: AtomicI64 = AtomicI64::new(0);

fn tick() -> Timestamp {
    Utc.timestamp_opt(1_800_000_000 + CLOCK.fetch_add(1, Ordering::Relaxed) + 1, 0).unwrap()
}

/// A fresh profile for seeded user `who`, as sign-up would make one.
fn profile_of(user: UserId, who: usize) -> UserProfile {
    let now = tick();
    UserProfile {
        id: new_id(),
        user_id: user,
        username: format!("user-{}", new_id().simple()),
        first_name: String::new(),
        last_name: String::new(),
        display_name: format!("user {who}"),
        avatar: String::new(),
        cover_image: None,
        timezone: "UTC".into(),
        date_joined: now,
        last_login_at: None,
        is_onboarded: false,
        is_tour_completed: false,
        last_workspace_id: None,
        onboarding_step: serde_json::json!({"profile_complete": false}),
        preferences: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }
}

/// The patch an `Op::ProfileUpdate` of this shape sends, with fresh values where it sets one.
fn patch_for(sel: PatchSel, workspace: WorkspaceId) -> ProfilePatch {
    let fresh = || new_id().simple().to_string();
    let prefs = |v: serde_json::Value| v.as_object().cloned().unwrap_or_default();
    match sel {
        PatchSel::Names => ProfilePatch {
            first_name: Some(format!("first-{}", fresh())),
            last_name: Some(format!("last-{}", fresh())),
            display_name: Some(format!("display-{}", fresh())),
            avatar: Some(format!("avatar-{}", fresh())),
            timezone: Some("Asia/Kolkata".into()),
            ..ProfilePatch::default()
        },
        PatchSel::Cover { clear: true } => ProfilePatch { cover_image: Some(None), ..ProfilePatch::default() },
        PatchSel::Cover { clear: false } => ProfilePatch { cover_image: Some(Some(format!("https://example.test/{}", fresh()))), ..ProfilePatch::default() },
        PatchSel::Prefs(0) => ProfilePatch { preferences: prefs(serde_json::json!({"a": fresh(), "b": {"x": true}})), ..ProfilePatch::default() },
        PatchSel::Prefs(_) => ProfilePatch { preferences: prefs(serde_json::json!({"b": 2, "c": [fresh()]})), ..ProfilePatch::default() },
        PatchSel::Onboarding => ProfilePatch {
            onboarding_step: Some(serde_json::json!({"profile_complete": true, "step": fresh()})),
            ..ProfilePatch::default()
        },
        PatchSel::Flags => ProfilePatch { is_onboarded: Some(true), is_tour_completed: Some(true), ..ProfilePatch::default() },
        PatchSel::Workspace { clear: true } => ProfilePatch { last_workspace_id: Some(None), ..ProfilePatch::default() },
        PatchSel::Workspace { clear: false } => ProfilePatch { last_workspace_id: Some(Some(workspace)), ..ProfilePatch::default() },
        PatchSel::Login => ProfilePatch { last_login_at: Some(tick()), ..ProfilePatch::default() },
        PatchSel::Empty => ProfilePatch::default(),
    }
}

/// The spec of `update_profile`, written from the port's documentation: what the patch names is set, the preferences are
/// merged key by key, the onboarding steps are replaced whole, `Some(None)` clears, and `updated_at` is always the call's.
fn apply_patch(p: &mut UserProfile, patch: &ProfilePatch, at: Timestamp) {
    let ProfilePatch {
        first_name,
        last_name,
        display_name,
        avatar,
        cover_image,
        timezone,
        last_login_at,
        is_onboarded,
        is_tour_completed,
        last_workspace_id,
        onboarding_step,
        preferences,
    } = patch.clone();
    p.first_name = first_name.unwrap_or_else(|| p.first_name.clone());
    p.last_name = last_name.unwrap_or_else(|| p.last_name.clone());
    p.display_name = display_name.unwrap_or_else(|| p.display_name.clone());
    p.avatar = avatar.unwrap_or_else(|| p.avatar.clone());
    p.cover_image = cover_image.unwrap_or_else(|| p.cover_image.clone());
    p.timezone = timezone.unwrap_or_else(|| p.timezone.clone());
    p.last_login_at = last_login_at.or(p.last_login_at);
    p.is_onboarded = is_onboarded.unwrap_or(p.is_onboarded);
    p.is_tour_completed = is_tour_completed.unwrap_or(p.is_tour_completed);
    p.last_workspace_id = last_workspace_id.unwrap_or(p.last_workspace_id);
    p.onboarding_step = onboarding_step.unwrap_or_else(|| p.onboarding_step.clone());
    if let Some(map) = p.preferences.as_object_mut() {
        map.extend(preferences);
    }
    p.updated_at = at;
}

fn item(ws: WorkspaceId, p: ProjectId, a: Allocation, ext: Option<u8>) -> WorkItem {
    WorkItem {
        id: WorkItemId(new_id()),
        workspace_id: ws,
        project_id: p,
        sequence_id: a.sequence_id,
        sort_order: a.sort_order,
        name: format!("item {}", a.sequence_id),
        description: None,
        external_source: ext.map(|_| "gh".to_string()),
        external_id: ext.map(|k| format!("k{k}")),
        created_by: UserId(new_id()),
        created_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    }
}

impl Model {
    fn new(fix: Fix) -> Model {
        let created = tick();
        let session = |i: usize, ttl: i64| SessModel {
            new: NewSession {
                token_hash: fix.tokens[i],
                user_id: fix.users[i],
                created_at: created,
                expires_at: created + TimeDelta::seconds(ttl),
            },
            revoked: false,
            last_seen: created,
        };
        let sessions = vec![session(0, 1_000_000), session(1, 100)];
        // both keys belong to user 0, so a listing has two rows; key 1 expires 100 s after it was created
        let key = |i: usize, ttl: Option<i64>| {
            let created_at = created + TimeDelta::seconds(i as i64);
            KeyModel {
                new: NewApiKey {
                    id: fix.keys[i].0,
                    user_id: fix.users[0],
                    token_hash: fix.keys[i].1,
                    hash_version: 1,
                    prefix: format!("thrk_{i:04}"),
                    label: format!("key {i}"),
                    created_at,
                    expires_at: ttl.map(|t| created_at + TimeDelta::seconds(t)),
                },
                revoked: None,
                last_used: None,
            }
        };
        let api_keys = vec![key(0, None), key(1, Some(100))];
        Model {
            fix,
            sessions,
            active: [true; 3],
            api_keys,
            logins: Vec::new(),
            profiles: [None, None, None],
            member_active: [[true; 2]; 7],
            rows: [Vec::new(), Vec::new()],
            counter: [0, 0],
            outbox: Vec::new(),
            activity: Vec::new(),
            projected: BTreeSet::new(),
        }
    }

    /// The calls that create the case's logins, sessions, API keys and account links; their answers are all `()`.
    fn setup(&self) -> Vec<Call> {
        let mut calls = vec![Call::SeedUsers { users: self.fix.login_users() }];
        calls.extend(self.sessions.iter().map(|s| Call::InsertSession { s: s.new.clone() }));
        calls.extend(self.api_keys.iter().map(|k| Call::InsertApiKey { k: k.new.clone() }));
        calls.extend((0..2).map(|i| Call::LinkAccount { a: self.fix.account(i, i) }));
        calls
    }

    /// What `user_by_email` and `user_for_account` must return for seeded user `i` now.
    fn user_record(&self, i: usize) -> UserRecord {
        self.fix.login_user(i, self.active[i])
    }

    fn undispatched(&self) -> Vec<OutboxRow> {
        let mut rows: Vec<OutboxRow> = self.outbox.iter().filter(|(_, d)| !d).map(|(r, _)| r.clone()).collect();
        rows.sort_by_key(|r| (r.created_at, r.envelope.event_id));
        rows
    }

    /// One `api_key` lookup of key `key` (2 is a digest that was never inserted), at a time that is `when` against its
    /// expiry, by the digest it is stored under or (`v`) by its digest under another scheme. The answer the adapter must
    /// give, and the class of the step.
    fn key_lookup(&self, key: usize, when: When, v: Option<i16>) -> (TokenHash, Timestamp, Option<LiveApiKey>, Class) {
        let Some(m) = self.api_keys.get(key) else {
            return (self.fix.digest(key, v.unwrap_or(1)), tick(), None, Class::KeyUnknown);
        };
        let base = m.new.expires_at.unwrap_or(m.new.created_at + TimeDelta::seconds(1_000));
        let now = match when {
            When::Before => base - TimeDelta::seconds(1),
            When::AtExpiry => base,
            When::After => base + TimeDelta::seconds(1),
        };
        let hash = v.map_or(m.new.token_hash, |v| self.fix.digest(key, v));
        if v.is_some_and(|v| v != m.new.hash_version) {
            // a digest of another scheme belongs to no row, whatever state the key is in
            return (hash, now, None, Class::KeyOtherVersionHidden);
        }
        let unexpired = m.new.expires_at.is_none_or(|e| e > now);
        let owner_active = self.active[0];
        let class = match (m.revoked.is_some(), unexpired, owner_active) {
            (true, _, _) => Class::KeyRevokedHidden,
            (false, false, _) => Class::KeyExpiredHidden,
            (false, true, false) => Class::KeyUserInactiveHidden,
            (false, true, true) => Class::KeyLive,
        };
        let want = (class == Class::KeyLive).then(|| LiveApiKey {
            id: m.new.id,
            user_id: m.new.user_id,
            last_used_at: m.last_used,
            hash_version: m.new.hash_version,
        });
        (hash, now, want, class)
    }

    fn plan(&mut self, op: &Op) -> (Vec<Call>, Vec<Class>) {
        let mut calls = Vec::new();
        let mut classes = Vec::new();
        match op {
            Op::Tx { steps, end } => self.plan_tx(steps, *end, &mut calls, &mut classes),
            &Op::List { proj, ws, with_deleted } => {
                let s = slot(proj);
                let want = if ws == Ws::Own {
                    classes.push(if with_deleted { Class::ListedWithDeleted } else { Class::ListedVisible });
                    self.rows[s].iter().filter(|r| with_deleted || r.deleted_at.is_none()).cloned().collect()
                } else {
                    classes.push(Class::ListedWrongWorkspace);
                    Vec::new()
                };
                let vis = if with_deleted { Visibility::WithDeleted } else { Visibility::Visible };
                calls.push(Call::List { ws: self.fix.ws_for(proj, ws), p: self.fix.p[s], vis, want });
            }
            &Op::Get { proj, ws, pick, with_deleted } => {
                let s = slot(proj);
                let row = pick.filter(|_| !self.rows[s].is_empty()).map(|i| self.rows[s][i % self.rows[s].len()].clone());
                let id = row.as_ref().map_or(WorkItemId(new_id()), |r| r.id);
                let (want, class) = match (ws, row) {
                    (Ws::Other, _) => (None, Class::GotWrongWorkspace),
                    (Ws::Own, None) => (None, Class::GotUnknown),
                    (Ws::Own, Some(r)) if with_deleted => (Some(r), Class::GotWithDeleted),
                    (Ws::Own, Some(r)) if r.deleted_at.is_some() => (None, Class::GotDeletedHidden),
                    (Ws::Own, Some(r)) => (Some(r), Class::GotLive),
                };
                classes.push(class);
                let vis = if with_deleted { Visibility::WithDeleted } else { Visibility::Visible };
                calls.push(Call::Get { ws: self.fix.ws_for(proj, ws), p: self.fix.p[slot(proj)], id, vis, want });
            }
            &Op::Membership { proj, slug, user } => {
                let want = membership(&self.fix, proj, slug, user, self.member_active[user as usize][slot(proj)]);
                classes.push(match (slug, want) {
                    (Sl::Unknown, _) => Class::UnknownSlug,
                    (Sl::Other, _) => Class::NoSuchProject,
                    (Sl::Own, Membership::NoSuchProject) => Class::NotInWorkspace,
                    (Sl::Own, Membership::NotAMember) if matches!((proj, user), (P::A, U::StrayB)) => {
                        Class::StrayMemberIgnored
                    }
                    (Sl::Own, Membership::NotAMember) => Class::NotAMember,
                    (Sl::Own, Membership::Member { .. }) => Class::IsMember,
                });
                let (slug, p, u) = (self.fix.slug_for(proj, slug), self.fix.p[slot(proj)], self.fix.user(user));
                calls.push(Call::Membership { slug, p, user: u, want });
            }
            &Op::SessionLookup { sess, when } => {
                let (hash, now, want, class) = match self.sessions.get(sess) {
                    None => (self.fix.tokens[sess], tick(), None, Class::SessionUnknown),
                    Some(m) => {
                        let at = m.new.expires_at;
                        let now = match when {
                            When::Before => at - TimeDelta::seconds(1),
                            When::AtExpiry => at,
                            When::After => at + TimeDelta::seconds(1),
                        };
                        let user_active = self.active[sess];
                        let live = !m.revoked && m.new.expires_at > now && user_active;
                        let class = match (m.revoked, m.new.expires_at > now, user_active) {
                            (true, _, _) => Class::SessionRevokedHidden,
                            (false, false, _) => Class::SessionExpiredHidden,
                            (false, true, false) => Class::SessionUserInactiveHidden,
                            (false, true, true) => Class::SessionLive,
                        };
                        let want = live.then(|| LiveSession { user_id: m.new.user_id, last_seen_at: m.last_seen });
                        (m.new.token_hash, now, want, class)
                    }
                };
                classes.push(class);
                calls.push(Call::Session { hash, now, want });
            }
            &Op::SessionTouch { sess } => {
                let at = tick();
                classes.push(if sess < self.sessions.len() { Class::SessionTouched } else { Class::SessionUnknown });
                if let Some(m) = self.sessions.get_mut(sess) {
                    m.last_seen = at;
                }
                calls.push(Call::TouchSession { hash: self.fix.tokens[sess], at });
            }
            &Op::SessionRevoke { sess } => {
                let at = tick();
                classes.push(match self.sessions.get(sess) {
                    None => Class::SessionUnknown,
                    Some(m) if m.revoked => Class::SessionRevokedAgain,
                    Some(_) => Class::SessionRevoked,
                });
                if let Some(m) = self.sessions.get_mut(sess) {
                    m.revoked = true;
                }
                calls.push(Call::RevokeSession { hash: self.fix.tokens[sess], at });
            }
            &Op::SessionReinsert { sess } => {
                let t = tick();
                classes.push(Class::SessionReinsertIgnored);
                let again = NewSession {
                    token_hash: self.fix.tokens[sess],
                    user_id: self.fix.users[6],
                    created_at: t,
                    expires_at: t + TimeDelta::seconds(9_999_999),
                };
                // the model does not change: the row stays as it was, revoked or not
                calls.push(Call::InsertSession { s: again });
            }
            &Op::SetActive { who, active } => {
                classes.push(if active { Class::UserReactivated } else { Class::UserDeactivated });
                self.active[who] = active;
                calls.push(Call::SetActive { user: self.fix.users[who], active });
            }
            &Op::RevokeUserSessions { who } => {
                let at = tick();
                let user = self.fix.users[who];
                let mut want = 0;
                for m in self.sessions.iter_mut().filter(|m| m.new.user_id == user && !m.revoked) {
                    m.revoked = true;
                    want += 1;
                }
                classes.push(if want > 0 { Class::UserSessionsRevoked } else { Class::UserSessionsRevokedNone });
                calls.push(Call::RevokeUserSessions { user, at, want });
            }
            &Op::KeyLookup { key, when } => {
                let (hash, now, want, class) = self.key_lookup(key, when, None);
                classes.push(class);
                calls.push(Call::ApiKey { hash, now, want });
            }
            &Op::KeyLookupAs { key, v } => {
                let (hash, now, want, class) = self.key_lookup(key, When::Before, Some(i16::from(v)));
                classes.push(class);
                calls.push(Call::ApiKey { hash, now, want });
            }
            &Op::KeyRehash { key, stale_from, to } => {
                let to = i16::from(to);
                let new_hash = self.fix.digest(key, to);
                let id = self.fix.keys[key].0;
                match self.api_keys.get_mut(key) {
                    None => {
                        classes.push(Class::KeyRehashUnknown);
                        calls.push(Call::RehashApiKey { id, from: 1, new_hash, to, want: Rehash::Changed(false) });
                    }
                    Some(m) => {
                        let stored = m.new.hash_version;
                        // 1 -> 2, 2 -> 3, 3 -> 1: never the version it is at
                        let from = if stale_from { stored % 3 + 1 } else { stored };
                        if from != stored {
                            classes.push(Class::KeyRehashStale);
                            calls.push(Call::RehashApiKey { id, from, new_hash, to, want: Rehash::Changed(false) });
                        } else {
                            classes.push(if m.revoked.is_some() { Class::KeyRehashedWhileRevoked } else { Class::KeyRehashed });
                            m.new.token_hash = new_hash;
                            m.new.hash_version = to;
                            calls.push(Call::RehashApiKey { id, from, new_hash, to, want: Rehash::Changed(true) });
                        }
                    }
                }
            }
            &Op::KeyRehashCollide { key } => {
                let other = self.api_keys[1 - key].new.token_hash;
                let m = &self.api_keys[key];
                classes.push(Class::KeyRehashConflict);
                calls.push(Call::RehashApiKey {
                    id: m.new.id,
                    from: m.new.hash_version,
                    new_hash: other,
                    to: if m.new.hash_version == 1 { 2 } else { 1 },
                    want: Rehash::Conflict,
                });
                // the key is where it was
                let (hash, now, want, class) = self.key_lookup(key, When::Before, None);
                classes.push(class);
                calls.push(Call::ApiKey { hash, now, want });
            }
            &Op::KeyMigration { key, to } => {
                let stored = self.api_keys[key].new.hash_version;
                let others: Vec<u8> = (1..=3u8).filter(|v| i16::from(*v) != stored).collect();
                let to = if i16::from(to) == stored { others[0] } else { to };
                self.plan_all(
                    &[
                        Op::KeyRehash { key, stale_from: false, to },
                        Op::KeyLookupAs { key, v: stored as u8 },
                        Op::KeyLookupAs { key, v: to },
                        Op::KeyRehash { key, stale_from: true, to: stored as u8 },
                        Op::KeyTouch { key },
                        Op::KeyLookupAs { key, v: to },
                        Op::KeyRehash { key, stale_from: false, to: stored as u8 },
                        Op::KeyLookupAs { key, v: stored as u8 },
                        Op::KeyLookupAs { key, v: to },
                        Op::KeyList { who: 0 },
                    ],
                    &mut calls,
                    &mut classes,
                );
            }
            &Op::KeyTouch { key } => {
                let at = tick();
                classes.push(if key < self.api_keys.len() { Class::KeyTouched } else { Class::KeyUnknown });
                if let Some(m) = self.api_keys.get_mut(key) {
                    m.last_used = Some(at);
                }
                calls.push(Call::TouchApiKey { id: self.fix.keys[key].0, at });
            }
            &Op::KeyRevoke { key, by_owner } => {
                let at = tick();
                let user = self.fix.users[if by_owner { 0 } else { 1 }];
                let want = match self.api_keys.get_mut(key) {
                    None => {
                        classes.push(Class::KeyUnknown);
                        false
                    }
                    Some(_) if !by_owner => {
                        classes.push(Class::KeyRevokeRefused);
                        false
                    }
                    Some(m) => {
                        classes.push(if m.revoked.is_some() { Class::KeyRevokedAgain } else { Class::KeyRevoked });
                        m.revoked.get_or_insert(at);
                        true
                    }
                };
                calls.push(Call::RevokeApiKey { user, id: self.fix.keys[key].0, at, want });
            }
            &Op::KeyList { who } => {
                let user = self.fix.users[who];
                let mut want: Vec<ApiKeyInfo> = self
                    .api_keys
                    .iter()
                    .filter(|m| m.new.user_id == user)
                    .map(|m| ApiKeyInfo {
                        id: m.new.id,
                        prefix: m.new.prefix.clone(),
                        label: m.new.label.clone(),
                        created_at: m.new.created_at,
                        expires_at: m.new.expires_at,
                        revoked_at: m.revoked,
                        last_used_at: m.last_used,
                    })
                    .collect();
                want.sort_by_key(|k| (k.created_at, k.id));
                classes.push(Class::KeyListed);
                calls.push(Call::ListApiKeys { user, want });
            }
            &Op::EmailLookup { who } => {
                let (email, want) = if who < 3 {
                    classes.push(if self.active[who] { Class::EmailFound } else { Class::EmailFoundInactive });
                    (self.fix.emails[who].clone(), Some(self.user_record(who)))
                } else {
                    classes.push(Class::EmailUnknown);
                    (format!("nobody-{}@example.test", new_id().simple()), None)
                };
                calls.push(Call::UserByEmail { email, want });
            }
            &Op::AccountLookup { who } => {
                let (sub, want) = if who < 2 {
                    classes.push(if self.active[who] { Class::AccountFound } else { Class::AccountFoundInactive });
                    (format!("sub-{who}"), Some(self.user_record(who)))
                } else {
                    classes.push(Class::AccountUnlinked);
                    (format!("sub-{who}"), None)
                };
                calls.push(Call::UserForAccount { provider: self.fix.provider.clone(), sub, want });
            }
            &Op::Deactivated { who, restore } => {
                let mut ops = vec![
                    Op::SetActive { who, active: false },
                    Op::SessionLookup { sess: who, when: When::Before },
                    Op::EmailLookup { who },
                    Op::AccountLookup { who },
                ];
                if who == 0 {
                    ops.push(Op::KeyLookup { key: 0, when: When::Before });
                }
                if restore {
                    ops.push(Op::SetActive { who, active: true });
                    ops.push(Op::SessionLookup { sess: who, when: When::Before });
                }
                self.plan_all(&ops, &mut calls, &mut classes);
            }
            &Op::OidcRoundTrip { ttl } => {
                self.plan_all(&[Op::OidcInsert { ttl }], &mut calls, &mut classes);
                let last = self.logins.len() - 1;
                self.plan_all(
                    &[Op::OidcTake { pick: Some(last) }, Op::OidcTake { pick: Some(last) }, Op::OidcTake { pick: None }],
                    &mut calls,
                    &mut classes,
                );
            }
            Op::OidcExpiry => {
                self.plan_all(&[Op::OidcInsert { ttl: 5 }], &mut calls, &mut classes);
                let first = self.logins.len() - 1;
                // each of these ticks the clock once (and touches nothing that matters here)
                let ticks: Vec<Op> = (0..6).map(|_| Op::KeyTouch { key: 2 }).collect();
                self.plan_all(&ticks, &mut calls, &mut classes);
                self.plan_all(&[Op::OidcInsert { ttl: 100 }], &mut calls, &mut classes);
                let second = self.logins.len() - 1;
                self.plan_all(
                    &[Op::OidcTake { pick: Some(first) }, Op::OidcTake { pick: Some(second) }],
                    &mut calls,
                    &mut classes,
                );
            }
            &Op::KeyCycle { key } => {
                self.plan_all(
                    &[
                        Op::KeyLookup { key, when: When::Before },
                        Op::KeyTouch { key },
                        Op::KeyLookup { key, when: When::Before },
                        Op::KeyTouch { key },
                        Op::KeyLookup { key, when: When::Before },
                        Op::KeyRevoke { key, by_owner: false },
                        Op::KeyLookup { key, when: When::Before },
                        Op::KeyRevoke { key, by_owner: true },
                        Op::KeyLookup { key, when: When::Before },
                        Op::KeyRevoke { key, by_owner: true },
                        Op::KeyList { who: 0 },
                    ],
                    &mut calls,
                    &mut classes,
                );
            }
            Op::AccountRelink => {
                classes.push(Class::AccountRelinkKept);
                // subject 0 is user 0's: linking it to user 2 must change nothing
                calls.push(Call::LinkAccount { a: self.fix.account(0, 2) });
                calls.push(Call::UserForAccount {
                    provider: self.fix.provider.clone(),
                    sub: "sub-0".into(),
                    want: Some(self.user_record(0)),
                });
            }
            &Op::UserInsert { kind } => {
                let fresh_email = format!("signup-{}@example.test", new_id().simple());
                let user_with = |id: UserId, email: String| UserRecord {
                    id,
                    email: Some(email),
                    password_hash: Some("$argon2id$stub-signup".into()),
                    is_active: true,
                };
                match kind {
                    UserIns::New => {
                        classes.push(Class::UserInserted);
                        let u = user_with(UserId(new_id()), fresh_email.clone());
                        calls.push(Call::InsertUser { u: u.clone(), conflict: false });
                        calls.push(Call::UserByEmail { email: fresh_email, want: Some(u) });
                    }
                    UserIns::EmailTaken(who) => {
                        classes.push(Class::UserInsertEmailTaken);
                        let u = user_with(UserId(new_id()), self.fix.emails[who].clone());
                        calls.push(Call::InsertUser { u: u.clone(), conflict: true });
                        // the holder is as it was: nothing was overwritten
                        calls.push(Call::UserByEmail { email: self.fix.emails[who].clone(), want: Some(self.user_record(who)) });
                    }
                    UserIns::IdTaken(who) => {
                        classes.push(Class::UserInsertIdTaken);
                        let u = user_with(self.fix.users[who], fresh_email.clone());
                        calls.push(Call::InsertUser { u, conflict: true });
                        calls.push(Call::UserByEmail { email: fresh_email, want: None });
                        calls.push(Call::UserByEmail { email: self.fix.emails[who].clone(), want: Some(self.user_record(who)) });
                    }
                }
            }
            &Op::ProfileInsert { who } => {
                let user = self.fix.users[who];
                match self.profiles[who].clone() {
                    None => {
                        let p = profile_of(user, who);
                        classes.push(Class::ProfileInserted);
                        calls.push(Call::InsertProfile { p: p.clone(), want: true });
                        calls.push(Call::Profile { user, want: Some(p.clone()) });
                        self.profiles[who] = Some(p);
                    }
                    Some(kept) => {
                        classes.push(Class::ProfileInsertKept);
                        // another profile for the same user: refused, and the one that is there is as it was
                        calls.push(Call::InsertProfile { p: profile_of(user, who), want: false });
                        calls.push(Call::Profile { user, want: Some(kept) });
                    }
                }
            }
            &Op::ProfileUpdate { who, sel } => {
                let user = self.fix.users[who];
                let patch = patch_for(sel, self.fix.ws[0]);
                let at = tick();
                match &mut self.profiles[who] {
                    None => {
                        classes.push(Class::ProfileUpdateMissing);
                        calls.push(Call::UpdateProfile { user, patch, at, want: None });
                        calls.push(Call::Profile { user, want: None });
                    }
                    Some(p) => {
                        classes.push(Class::ProfileUpdated);
                        if p.preferences.as_object().is_some_and(|held| held.keys().any(|k| !patch.preferences.contains_key(k))) && !patch.preferences.is_empty() {
                            classes.push(Class::ProfilePrefsMerged);
                        }
                        apply_patch(p, &patch, at);
                        let want = p.clone();
                        calls.push(Call::UpdateProfile { user, patch, at, want: Some(want.clone()) });
                        calls.push(Call::Profile { user, want: Some(want) });
                    }
                }
            }
            &Op::ProfileRead { who } => {
                let want = self.profiles[who].clone();
                calls.push(Call::Profile { user: self.fix.users[who], want });
            }
            &Op::ProfileLife { who } => {
                let update = |sel| Op::ProfileUpdate { who, sel };
                let mut ops = Vec::new();
                if self.profiles[who].is_none() {
                    ops.push(Op::ProfileInsert { who });
                }
                ops.extend([
                    update(PatchSel::Prefs(0)),
                    update(PatchSel::Prefs(1)),
                    update(PatchSel::Cover { clear: false }),
                    update(PatchSel::Cover { clear: true }),
                    update(PatchSel::Onboarding),
                    update(PatchSel::Flags),
                    update(PatchSel::Workspace { clear: false }),
                    update(PatchSel::Workspace { clear: true }),
                    update(PatchSel::Names),
                    update(PatchSel::Login),
                    update(PatchSel::Empty),
                    Op::ProfileRead { who },
                ]);
                self.plan_all(&ops, &mut calls, &mut classes);
            }
            &Op::WorkspaceLife { with_profile } => {
                let (owner, other) = (UserId(new_id()), UserId(new_id()));
                let login = |id| UserRecord {
                    id,
                    email: Some(format!("wl-{}@example.test", new_id().simple())),
                    password_hash: Some("$argon2id$stub-workspace".into()),
                    is_active: true,
                };
                let (owner_rec, other_rec) = (login(owner), login(other));
                calls.push(Call::InsertUser { u: owner_rec.clone(), conflict: false });
                calls.push(Call::InsertUser { u: other_rec, conflict: false });
                // what the member list says about the owner: the profile's fields, or nothing at all
                let profile = with_profile.then(|| profile_of(owner, 9));
                if let Some(p) = &profile {
                    calls.push(Call::InsertProfile { p: p.clone(), want: true });
                    classes.push(Class::WorkspaceMemberProfileShown);
                }
                let shown = MemberUser {
                    id: owner,
                    email: owner_rec.email.clone(),
                    first_name: profile.as_ref().map(|p| p.first_name.clone()).unwrap_or_default(),
                    last_name: profile.as_ref().map(|p| p.last_name.clone()).unwrap_or_default(),
                    display_name: profile.as_ref().map(|p| p.display_name.clone()).unwrap_or_default(),
                    avatar: profile.as_ref().map(|p| p.avatar.clone()).unwrap_or_default(),
                };
                let scope = |w: &NewWorkspace, user, role| (w.workspace.id, user, role);

                let first = new_workspace(owner, Role::Admin, "wl-a");
                let ws1 = &first.workspace;
                let listing1 = WorkspaceListing { workspace: ws1.clone(), role: Role::Admin, total_members: 1 };
                calls.push(Call::SlugTaken { slug: ws1.slug.as_str().to_owned(), want: false });
                calls.push(Call::AnyWorkspace { want: true });
                calls.push(Call::InsertWorkspace { w: first.clone(), conflict: false });
                classes.push(Class::WorkspaceMade);
                calls.push(Call::AnyWorkspace { want: true });
                calls.push(Call::SlugTaken { slug: ws1.slug.as_str().to_owned(), want: true });
                // the slug check is exact: another case is another slug
                calls.push(Call::SlugTaken { slug: ws1.slug.as_str().to_uppercase(), want: false });
                calls.push(Call::WorkspacesOf { user: owner, want: vec![listing1.clone()] });
                calls.push(Call::WorkspacesOf { user: other, want: vec![] });
                let access = |w: &NewWorkspace, role| Some(WorkspaceAccess { workspace_id: w.workspace.id, role });
                calls.push(Call::WorkspaceAccessOf { slug: ws1.slug.clone(), user: owner, want: access(&first, Role::Admin) });
                calls.push(Call::WorkspaceAccessOf { slug: ws1.slug.clone(), user: other, want: None });
                calls.push(Call::WorkspaceAccessOf { slug: WorkspaceSlug::new("no-such-workspace"), user: owner, want: None });
                calls.push(Call::WorkspaceMemberOf { scope: scope(&first, owner, Role::Admin), want: Some(first.owner.clone()) });
                calls.push(Call::WorkspaceMemberOf { scope: scope(&first, other, Role::Member), want: None });
                calls.push(Call::WorkspaceMembersOf {
                    scope: scope(&first, owner, Role::Admin),
                    want: vec![MemberWithUser { member: first.owner.clone(), user: shown.clone() }],
                });

                // a second workspace, made later: the list is newest first
                let second = new_workspace(owner, Role::Member, "wl-b");
                let listing2 = WorkspaceListing { workspace: second.workspace.clone(), role: Role::Member, total_members: 1 };
                calls.push(Call::InsertWorkspace { w: second.clone(), conflict: false });
                classes.push(Class::WorkspaceMade);
                calls.push(Call::WorkspacesOf { user: owner, want: vec![listing2.clone(), listing1.clone()] });

                // a creation that loses the race for a slug is refused and leaves nothing of its own
                let mut slug_loser = new_workspace(other, Role::Admin, "wl-c");
                slug_loser.workspace.slug = ws1.slug.clone();
                calls.push(Call::InsertWorkspace { w: slug_loser, conflict: true });
                // a creation whose member row cannot be written (its id is taken) takes its workspace back with it
                let mut member_loser = new_workspace(other, Role::Admin, "wl-d");
                member_loser.owner.id = first.owner.id;
                let loser_slug = member_loser.workspace.slug.as_str().to_owned();
                calls.push(Call::InsertWorkspace { w: member_loser, conflict: true });
                classes.push(Class::WorkspaceSlugConflict);
                calls.push(Call::SlugTaken { slug: loser_slug, want: false });
                calls.push(Call::WorkspacesOf { user: other, want: vec![] });
                calls.push(Call::WorkspacesOf { user: owner, want: vec![listing2.clone(), listing1.clone()] });

                // deactivated, the workspace is gone from the owner's list and slug lookup but not from the member list
                let mut suspended = first.owner.clone();
                suspended.is_active = false;
                calls.push(Call::SetMemberActive { ws: ws1.id, user: owner, active: false, want: true });
                classes.push(Class::WorkspaceMemberDeactivated);
                calls.push(Call::WorkspacesOf { user: owner, want: vec![listing2.clone()] });
                calls.push(Call::WorkspaceAccessOf { slug: ws1.slug.clone(), user: owner, want: None });
                calls.push(Call::SlugTaken { slug: ws1.slug.as_str().to_owned(), want: true });
                calls.push(Call::WorkspaceMemberOf { scope: scope(&first, owner, Role::Admin), want: Some(suspended.clone()) });
                calls.push(Call::WorkspaceMembersOf {
                    scope: scope(&first, owner, Role::Admin),
                    want: vec![MemberWithUser { member: suspended, user: shown }],
                });
                // a membership that does not exist cannot be switched
                calls.push(Call::SetMemberActive { ws: ws1.id, user: other, active: false, want: false });
                calls.push(Call::SetMemberActive { ws: ws1.id, user: owner, active: true, want: true });
                classes.push(Class::WorkspaceMemberReactivated);
                calls.push(Call::WorkspacesOf { user: owner, want: vec![listing2, listing1] });
                calls.push(Call::WorkspaceAccessOf { slug: ws1.slug.clone(), user: owner, want: access(&first, Role::Admin) });
            }
            &Op::ProjectRolesRead { proj, user } => {
                let s = slot(proj);
                let want: Vec<(ProjectId, Role)> = role_in(proj, user).map(|r| (self.fix.p[s], r)).into_iter().collect();
                classes.push(if want.is_empty() { Class::ProjectRolesNone } else { Class::ProjectRolesSome });
                calls.push(Call::ProjectRolesOf { scope: (self.fix.ws[s], self.fix.user(user), Role::Member), want });
            }
            &Op::MemberActive { ws, user, active } => {
                let s = slot(ws);
                let member = in_workspace(user)[s];
                let was = self.member_active[user as usize][s];
                calls.push(Call::SetMemberActive { ws: self.fix.ws[s], user: self.fix.user(user), active, want: member });
                if member {
                    self.member_active[user as usize][s] = active;
                    if !active {
                        classes.push(Class::WorkspaceMemberDeactivated);
                    } else if !was {
                        classes.push(Class::WorkspaceMemberReactivated);
                    }
                }
                // seeded members hold the role `member`; only active ones get in by slug or see the workspace listed
                let access = (member && self.member_active[user as usize][s])
                    .then_some(WorkspaceAccess { workspace_id: self.fix.ws[s], role: Role::Member });
                calls.push(Call::WorkspaceAccessOf { slug: self.fix.slug[s].clone(), user: self.fix.user(user), want: access });
                for u in U::ALL {
                    let want = (0..2)
                        .filter(|&w| in_workspace(u)[w] && self.member_active[u as usize][w])
                        .map(|w| {
                            let total = U::ALL.iter().filter(|&&n| in_workspace(n)[w] && self.member_active[n as usize][w]).count();
                            (self.fix.ws[w], Role::Member, total as u64)
                        })
                        .collect::<Vec<_>>();
                    calls.push(Call::SeededListing { user: self.fix.user(u), want: sorted_by_id(want, |t| t.0 .0) });
                }
                let members: Vec<(UserId, bool)> = U::ALL
                    .iter()
                    .filter(|&&u| in_workspace(u)[s])
                    .map(|&u| (self.fix.user(u), self.member_active[u as usize][s]))
                    .collect();
                calls.push(Call::SeededMembers { ws: self.fix.ws[s], want: sorted_by_id(members, |t| t.0 .0) });
            }
            &Op::OidcInsert { ttl } => {
                let created_at = tick();
                let l = NewOidcLogin {
                    state_hash: token_hash(),
                    nonce: format!("nonce-{}", new_id().simple()),
                    code_verifier: format!("verifier-{}", new_id().simple()),
                    created_at,
                    expires_at: created_at + TimeDelta::seconds(i64::from(ttl)),
                };
                let mut swept = false;
                for (old, state) in &mut self.logins {
                    if *state == LoginState::Pending && old.expires_at <= created_at {
                        *state = LoginState::Swept;
                        swept = true;
                    }
                }
                classes.push(Class::OidcInserted);
                if swept {
                    classes.push(Class::OidcSwept);
                }
                self.logins.push((l.clone(), LoginState::Pending));
                calls.push(Call::InsertOidcLogin { l });
            }
            &Op::OidcTake { pick } => {
                let n = self.logins.len();
                match pick.filter(|_| n > 0).map(|i| i % n) {
                    Some(i) => {
                        let (l, state) = &mut self.logins[i];
                        let (want, class) = match *state {
                            LoginState::Pending => (
                                Some(OidcLogin { nonce: l.nonce.clone(), code_verifier: l.code_verifier.clone(), expires_at: l.expires_at }),
                                Class::OidcTaken,
                            ),
                            LoginState::Taken => (None, Class::OidcReplayHidden),
                            LoginState::Swept => (None, Class::OidcSweptHidden),
                        };
                        *state = if *state == LoginState::Pending { LoginState::Taken } else { *state };
                        classes.push(class);
                        calls.push(Call::TakeOidcLogin { state: l.state_hash, want });
                    }
                    None => {
                        classes.push(Class::OidcUnknown);
                        calls.push(Call::TakeOidcLogin { state: token_hash(), want: None });
                    }
                }
            }
            &Op::Undispatched { limit } => {
                let want: Vec<OutboxRow> = self.undispatched().into_iter().take(limit as usize).collect();
                if !want.is_empty() {
                    classes.push(Class::UndispatchedSome);
                }
                calls.push(Call::Undispatched { limit, want });
            }
            &Op::Mark { pick } => {
                let at = tick();
                let n = self.outbox.len();
                match pick.filter(|_| n > 0).map(|i| i % n) {
                    Some(i) => {
                        classes.push(if self.outbox[i].1 { Class::MarkedAgain } else { Class::Marked });
                        self.outbox[i].1 = true;
                        calls.push(Call::Mark { event: self.outbox[i].0.envelope.event_id, at });
                    }
                    None => {
                        classes.push(Class::MarkedUnknown);
                        calls.push(Call::Mark { event: EventId(new_id()), at });
                    }
                }
            }
            Op::Depth => {
                classes.push(Class::Depth);
                calls.push(Call::Depth { want: self.undispatched().len() as u64 });
            }
            &Op::InsertActivity { proj, event } => {
                let s = slot(proj);
                let projected: Vec<EventId> = self.activity.iter().map(|a| a.event_id).collect();
                let event_id = match event {
                    Ev::Outbox(i) if !self.outbox.is_empty() => self.outbox[i % self.outbox.len()].0.envelope.event_id,
                    Ev::Again(i) if !projected.is_empty() => projected[i % projected.len()],
                    _ => EventId(new_id()),
                };
                let entity = if self.rows[s].is_empty() { new_id() } else { self.rows[s][0].id.0 };
                let a = Activity {
                    id: ActivityId(new_id()),
                    event_id,
                    workspace_id: self.fix.ws[s],
                    project_id: self.fix.p[s],
                    entity_id: entity,
                    verb: "created".into(),
                    actor_id: Some(self.fix.users[0]),
                    created_at: tick(),
                };
                let want = self.projected.insert(event_id);
                classes.push(if want { Class::ActivityInserted } else { Class::ActivityRedelivered });
                if want {
                    self.activity.push(a.clone());
                }
                calls.push(Call::InsertActivity { a, want });
            }
            &Op::ListActivity { proj, ws, entity } => {
                let s = slot(proj);
                let entity = entity.filter(|_| !self.rows[s].is_empty()).map(|i| self.rows[s][i % self.rows[s].len()].id);
                let want: Vec<Activity> = if ws == Ws::Own {
                    classes.push(if entity.is_some() { Class::ActivityListedForEntity } else { Class::ActivityListed });
                    self.activity
                        .iter()
                        .filter(|a| a.project_id == self.fix.p[s] && entity.is_none_or(|e| a.entity_id == e.0))
                        .cloned()
                        .collect()
                } else {
                    classes.push(Class::ActivityWrongWorkspace);
                    Vec::new()
                };
                calls.push(Call::ListActivity { ws: self.fix.ws_for(proj, ws), p: self.fix.p[s], entity, want });
            }
        }
        (calls, classes)
    }

    /// Plan `ops` one after the other, appending their calls and classes.
    fn plan_all(&mut self, ops: &[Op], calls: &mut Vec<Call>, classes: &mut Vec<Class>) {
        for op in ops {
            let (c, k) = self.plan(op);
            calls.extend(c);
            classes.extend(k);
        }
    }

    fn plan_tx(&mut self, steps: &[Step], end: End, calls: &mut Vec<Call>, classes: &mut Vec<Class>) {
        calls.push(Call::Begin { ws: self.fix.ws[0], p: self.fix.p[0] });
        let mut allocs = [0i64; 2];
        // rows written by this transaction, and the ids it soft-deleted (committed rows or its own)
        let mut written: [Vec<WorkItem>; 2] = [Vec::new(), Vec::new()];
        let mut deleted: Vec<(usize, WorkItemId, Timestamp)> = Vec::new();
        let mut outbox: Vec<OutboxRow> = Vec::new();
        let is_deleted = |deleted: &[(usize, WorkItemId, Timestamp)], id: WorkItemId| deleted.iter().any(|d| d.1 == id);
        let mut failed = false;
        for st in steps {
            match *st {
                Step::Create { proj, ext } => {
                    let s = slot(proj);
                    allocs[s] += 1;
                    let seq = self.counter[s] + allocs[s];
                    let a = Allocation { sequence_id: seq, sort_order: seq * 10_000 };
                    calls.push(Call::Allocate { ws: self.fix.ws[s], p: self.fix.p[s], want: Some(a) });
                    let it = item(self.fix.ws[s], self.fix.p[s], a, ext);
                    let key = it.external_id.clone();
                    let live_in = |rows: &[WorkItem]| {
                        rows.iter().any(|r| {
                            key.is_some() && r.external_id == key && r.deleted_at.is_none() && !is_deleted(&deleted, r.id)
                        })
                    };
                    let (in_committed, in_tx) = (live_in(&self.rows[s]), live_in(&written[s]));
                    if in_committed || in_tx {
                        classes.push(if in_tx { Class::ConflictWithinTx } else { Class::Conflict });
                        calls.push(Call::Insert { item: it, conflict: true });
                        failed = true;
                        break;
                    }
                    let freed = key.is_some() && self.rows[s].iter().chain(&written[s]).any(|r| r.external_id == key);
                    classes.push(if freed { Class::ExternalKeyFreed } else { Class::Created });
                    calls.push(Call::Insert { item: it.clone(), conflict: false });
                    written[s].push(it);
                }
                Step::Delete { proj, ws, pick } => {
                    let s = slot(proj);
                    let candidates: Vec<(&WorkItem, bool)> =
                        self.rows[s].iter().map(|r| (r, false)).chain(written[s].iter().map(|r| (r, true))).collect();
                    let target = (!candidates.is_empty()).then(|| candidates[pick % candidates.len()]);
                    let id = target.map_or(WorkItemId(new_id()), |(r, _)| r.id);
                    let live = target.is_some_and(|(r, _)| r.deleted_at.is_none() && !is_deleted(&deleted, r.id));
                    let want = ws == Ws::Own && live;
                    classes.push(match (ws, target) {
                        (Ws::Other, _) => Class::DeleteWrongWorkspace,
                        (Ws::Own, None) => Class::DeleteUnknown,
                        (Ws::Own, Some(_)) if !live => Class::DeleteAlreadyDeleted,
                        (Ws::Own, Some((_, true))) => Class::DeletedWithinTx,
                        (Ws::Own, Some((_, false))) => Class::DeletedCommitted,
                    });
                    let at = tick();
                    calls.push(Call::SoftDelete { ws: self.fix.ws_for(proj, ws), p: self.fix.p[s], id, at, want });
                    if want {
                        deleted.push((s, id, at));
                    }
                }
                Step::Outbox { proj } => {
                    let s = slot(proj);
                    let at = tick();
                    let row = OutboxRow {
                        envelope: Envelope {
                            event_id: EventId(new_id()),
                            schema_version: 1,
                            workspace_id: self.fix.ws[s],
                            event_type: "work_item.created".into(),
                            entity_type: "work_item".into(),
                            entity_id: new_id(),
                            actor_id: Some(self.fix.users[0]),
                            occurred_at: at,
                            payload: serde_json::json!({ "project_id": self.fix.p[s] }),
                            trace_id: None,
                        },
                        created_at: at,
                        dispatched_at: None,
                        attempts: 0,
                    };
                    classes.push(Class::OutboxWritten);
                    calls.push(Call::InsertOutbox { row: row.clone() });
                    outbox.push(row);
                }
                Step::AllocateElsewhere { proj } => {
                    classes.push(Class::AllocateWrongWorkspace);
                    calls.push(Call::Allocate { ws: self.fix.ws_for(proj, Ws::Other), p: self.fix.p[slot(proj)], want: None });
                    failed = true;
                    break;
                }
            }
        }
        // FR-22: every number `allocate` handed out is taken for good, whatever becomes of the transaction.
        for s in 0..2 {
            self.counter[s] += allocs[s];
        }
        let took_numbers = allocs.iter().any(|a| *a > 0);
        if failed {
            classes.push(Class::FailedAndRolledBack);
            if took_numbers {
                classes.push(Class::NumbersKeptAfterAbort);
            }
            calls.push(Call::Rollback);
            return;
        }
        match end {
            End::Commit => {
                classes.push(Class::Committed);
                calls.push(Call::Commit);
                for s in 0..2 {
                    self.rows[s].append(&mut written[s]);
                }
                for (s, id, at) in deleted {
                    if let Some(r) = self.rows[s].iter_mut().find(|r| r.id == id) {
                        r.deleted_at = Some(at);
                    }
                }
                self.outbox.extend(outbox.into_iter().map(|r| (r, false)));
            }
            End::Rollback => {
                classes.push(Class::RolledBack);
                if took_numbers {
                    classes.push(Class::NumbersKeptAfterAbort);
                }
                calls.push(Call::Rollback);
            }
            End::Abandon => {
                classes.push(Class::Abandoned);
                if took_numbers {
                    classes.push(Class::NumbersKeptAfterAbort);
                }
                calls.push(Call::Abandon);
            }
        }
    }

    /// The calls that end a case: every outbox row of the case marked dispatched, and an empty outbox.
    fn drain(&mut self) -> Vec<Call> {
        let at = tick();
        let mut calls: Vec<Call> = self
            .outbox
            .iter_mut()
            .filter(|(_, d)| !d)
            .map(|(r, d)| {
                *d = true;
                Call::Mark { event: r.envelope.event_id, at }
            })
            .collect();
        calls.push(Call::Depth { want: 0 });
        calls
    }
}

// ---------------------------------------------------------------------------------------------
// Running a script against Postgres
// ---------------------------------------------------------------------------------------------

fn fail(at: &str, e: impl std::fmt::Display) -> TestCaseError {
    TestCaseError::fail(format!("{at}: {e}"))
}

/// Every port call is bounded: a wedged pool (a recycled connection still inside a transaction, holding the counter
/// row's lock) must fail the case, not hang it.
async fn bounded<T>(at: &str, f: impl std::future::Future<Output = Result<T, DbError>>) -> Result<T, TestCaseError> {
    match tokio::time::timeout(Duration::from_secs(20), f).await {
        Ok(r) => r.map_err(|e| fail(at, format!("{e:?}"))),
        Err(_) => Err(fail(at, "timed out after 20 s (a wedged pool or a held lock)")),
    }
}

/// The proof a call passes. The adapter reads only the tenant key from it, so the user is a placeholder; the
/// planner names the workspace itself, the other tenant's included, to reach the adapter's own checks.
fn scope(ws: WorkspaceId, p: ProjectId) -> Scope {
    Scope::mint(ws, p, UserId(Uuid::nil()))
}

fn open<'a>(tx: &'a mut Option<Box<dyn Tx>>, at: &str) -> Result<&'a mut (dyn Tx + 'static), TestCaseError> {
    tx.as_deref_mut().ok_or_else(|| fail(at, "no open transaction"))
}

async fn execute(db: &dyn Database, tx: &mut Option<Box<dyn Tx>>, call: Call, at: &str) -> Result<(), TestCaseError> {
    let open = |tx| open(tx, at);
    let sys = SystemScope::mint();
    match call {
        Call::Begin { ws, p } => *tx = Some(bounded(at, db.begin(&scope(ws, p))).await?),
        Call::Allocate { ws, p, want: Some(want) } => {
            let got = bounded(at, open(tx)?.allocate(&scope(ws, p))).await?;
            prop_assert_eq!(got, want, "{}: allocation", at);
        }
        Call::Allocate { ws, p, want: None } => {
            let got = tokio::time::timeout(Duration::from_secs(20), open(tx)?.allocate(&scope(ws, p)))
                .await
                .map_err(|_| fail(at, "timed out after 20 s"))?;
            if let Ok(a) = got {
                return Err(fail(at, format!("allocate under another workspace: got {a:?}, want a refusal")));
            }
        }
        Call::Insert { item, conflict } => {
            let got = tokio::time::timeout(Duration::from_secs(20), open(tx)?.insert_work_item(&scope(item.workspace_id, item.project_id), &item))
                .await
                .map_err(|_| fail(at, "timed out after 20 s"))?;
            match (got, conflict) {
                (Ok(()), false) | (Err(DbError::Conflict(_)), true) => {}
                (got, _) => return Err(fail(at, format!("insert: got {got:?}, want conflict = {conflict}"))),
            }
        }
        Call::SoftDelete { ws, p, id, at: when, want } => {
            let got = bounded(at, open(tx)?.soft_delete_work_item(&scope(ws, p), id, when)).await?;
            prop_assert_eq!(got, want, "{}: soft delete", at);
        }
        Call::InsertOutbox { row } => {
            let p = row.envelope.project_id().ok_or_else(|| fail(at, "outbox row without project_id"))?;
            bounded(at, open(tx)?.insert_outbox(&scope(row.envelope.workspace_id, p), &row)).await?
        }
        Call::Commit => bounded(at, tx.take().ok_or_else(|| fail(at, "no open transaction"))?.commit()).await?,
        Call::Rollback => bounded(at, tx.take().ok_or_else(|| fail(at, "no open transaction"))?.rollback()).await?,
        Call::Abandon => drop(tx.take()),
        Call::List { ws, p, vis, want } => {
            let got = bounded(at, db.list_work_items(&scope(ws, p), vis)).await?;
            prop_assert_eq!(got, want, "{}: list", at);
        }
        Call::Get { ws, p, id, vis, want } => {
            let got = bounded(at, db.get_work_item(&scope(ws, p), id, vis)).await?;
            prop_assert_eq!(got, want, "{}: get", at);
        }
        Call::Membership { slug, p, user, want } => {
            let got = bounded(at, db.membership(&slug, p, user)).await?;
            prop_assert_eq!(got, want, "{}: membership", at);
        }
        Call::InsertSession { s } => bounded(at, db.insert_session(&s)).await?,
        Call::Session { hash, now, want } => {
            let got = bounded(at, db.session(&hash, now)).await?;
            prop_assert_eq!(got, want, "{}: session", at);
        }
        Call::TouchSession { hash, at: when } => bounded(at, db.touch_session(&hash, when)).await?,
        Call::RevokeSession { hash, at: when } => bounded(at, db.revoke_session(&hash, when)).await?,
        Call::SeedUsers { users } => bounded(at, db.seed_users(&sys, &users)).await?,
        Call::InsertUser { u, conflict } => {
            let got = tokio::time::timeout(Duration::from_secs(20), db.insert_user(&u))
                .await
                .map_err(|_| fail(at, "timed out after 20 s"))?;
            match (got, conflict) {
                (Ok(()), false) | (Err(DbError::Conflict(_)), true) => {}
                (got, _) => return Err(fail(at, format!("insert user: got {got:?}, want conflict = {conflict}"))),
            }
        }
        Call::InsertProfile { p, want } => {
            let got = bounded(at, db.insert_profile(&p)).await?;
            prop_assert_eq!(got, want, "{}: insert profile (true = made)", at);
        }
        Call::Profile { user, want } => {
            let got = bounded(at, db.profile(user)).await?;
            prop_assert_eq!(got, want, "{}: profile", at);
        }
        Call::UpdateProfile { user, patch, at: when, want } => {
            let got = bounded(at, db.update_profile(user, &patch, when)).await?;
            prop_assert_eq!(got, want, "{}: update profile", at);
        }
        Call::InsertWorkspace { w, conflict } => {
            let got = tokio::time::timeout(Duration::from_secs(20), db.insert_workspace(&w))
                .await
                .map_err(|_| fail(at, "timed out after 20 s"))?;
            match (got, conflict) {
                (Ok(()), false) | (Err(DbError::Conflict(_)), true) => {}
                (got, _) => return Err(fail(at, format!("insert workspace: got {got:?}, want conflict = {conflict}"))),
            }
        }
        Call::SlugTaken { slug, want } => {
            let got = bounded(at, db.workspace_slug_taken(&slug)).await?;
            prop_assert_eq!(got, want, "{}: slug taken", at);
        }
        Call::AnyWorkspace { want } => {
            let got = bounded(at, db.any_workspace()).await?;
            prop_assert_eq!(got, want, "{}: any workspace", at);
        }
        Call::WorkspacesOf { user, want } => {
            let got = bounded(at, db.workspaces_of(user)).await?;
            prop_assert_eq!(got, want, "{}: workspaces of", at);
        }
        Call::WorkspaceAccessOf { slug, user, want } => {
            let got = bounded(at, db.workspace_access(&slug, user)).await?;
            prop_assert_eq!(got, want, "{}: workspace access", at);
        }
        Call::SetMemberActive { ws, user, active, want } => {
            let got = bounded(at, db.set_workspace_member_active(&sys, ws, user, active)).await?;
            prop_assert_eq!(got, want, "{}: set workspace member active (true = the membership exists)", at);
        }
        Call::WorkspaceMemberOf { scope: (w, u, r), want } => {
            let got = bounded(at, db.workspace_member(&WorkspaceScope::mint(w, u, r))).await?;
            prop_assert_eq!(got, want, "{}: workspace member", at);
        }
        Call::WorkspaceMembersOf { scope: (w, u, r), want } => {
            let got = bounded(at, db.workspace_members(&WorkspaceScope::mint(w, u, r))).await?;
            prop_assert_eq!(got, want, "{}: workspace members", at);
        }
        Call::ProjectRolesOf { scope: (w, u, r), want } => {
            let got = bounded(at, db.project_roles(&WorkspaceScope::mint(w, u, r))).await?;
            prop_assert_eq!(got, want, "{}: project roles", at);
        }
        Call::SeededListing { user, want } => {
            let got = bounded(at, db.workspaces_of(user)).await?;
            let got = sorted_by_id(got.into_iter().map(|l| (l.workspace.id, l.role, l.total_members)).collect(), |t| t.0 .0);
            prop_assert_eq!(got, want, "{}: seeded workspaces of", at);
        }
        Call::SeededMembers { ws, want } => {
            let got = bounded(at, db.workspace_members(&WorkspaceScope::mint(ws, UserId(Uuid::nil()), Role::Member))).await?;
            let got = sorted_by_id(got.into_iter().map(|m| (m.member.user_id, m.member.is_active)).collect(), |t| t.0 .0);
            prop_assert_eq!(got, want, "{}: seeded workspace members", at);
        }
        Call::LinkAccount { a } => bounded(at, db.link_account(&a)).await?,
        Call::InsertApiKey { k } => bounded(at, db.insert_api_key(&k)).await?,
        Call::SetActive { user, active } => {
            let found = bounded(at, db.set_user_active(&sys, user, active)).await?;
            prop_assert!(found, "{}: the user exists", at);
        }
        Call::RevokeUserSessions { user, at: when, want } => {
            let got = bounded(at, db.revoke_user_sessions(user, when)).await?;
            prop_assert_eq!(got, want, "{}: sessions revoked", at);
        }
        Call::ApiKey { hash, now, want } => {
            let got = bounded(at, db.api_key(&hash, now)).await?;
            prop_assert_eq!(got, want, "{}: api key", at);
        }
        Call::TouchApiKey { id, at: when } => bounded(at, db.touch_api_key(id, when)).await?,
        Call::RehashApiKey { id, from, new_hash, to, want } => {
            let got = tokio::time::timeout(Duration::from_secs(20), db.rehash_api_key(id, from, &new_hash, to))
                .await
                .map_err(|_| fail(at, "timed out after 20 s"))?;
            match (got, want) {
                (Ok(changed), Rehash::Changed(w)) if changed == w => {}
                (Err(DbError::Conflict(_)), Rehash::Conflict) => {}
                (got, want) => return Err(fail(at, format!("rehash api key: got {got:?}, want {want:?}"))),
            }
        }
        Call::RevokeApiKey { user, id, at: when, want } => {
            let got = bounded(at, db.revoke_api_key(user, id, when)).await?;
            prop_assert_eq!(got, want, "{}: revoke api key (true = the caller's key)", at);
        }
        Call::ListApiKeys { user, want } => {
            let got = bounded(at, db.list_api_keys(user)).await?;
            prop_assert_eq!(got, want, "{}: list api keys", at);
        }
        Call::UserByEmail { email, want } => {
            let got = bounded(at, db.user_by_email(&email)).await?;
            prop_assert_eq!(got, want, "{}: user by email", at);
        }
        Call::UserForAccount { provider, sub, want } => {
            let got = bounded(at, db.user_for_account(&provider, &sub)).await?;
            prop_assert_eq!(got, want, "{}: user for account", at);
        }
        Call::InsertOidcLogin { l } => bounded(at, db.insert_oidc_login(&l)).await?,
        Call::TakeOidcLogin { state, want } => {
            let got = bounded(at, db.take_oidc_login(&state)).await?;
            prop_assert_eq!(got, want, "{}: take oidc login", at);
        }
        Call::Undispatched { limit, want } => {
            let got = bounded(at, db.undispatched_outbox(&sys, limit)).await?;
            prop_assert_eq!(got, want, "{}: undispatched outbox", at);
        }
        Call::Mark { event, at: when } => bounded(at, db.mark_outbox_dispatched(&sys, event, when)).await?,
        Call::Depth { want } => {
            let got = bounded(at, db.outbox_depth(&sys)).await?;
            prop_assert_eq!(got, want, "{}: outbox depth", at);
        }
        Call::InsertActivity { a, want } => {
            let got = bounded(at, db.insert_activity(&sys, &a)).await?;
            prop_assert_eq!(got, want, "{}: insert activity (true = new event id)", at);
        }
        Call::ListActivity { ws, p, entity, want } => {
            let got = bounded(at, db.list_activity(&scope(ws, p), entity)).await?;
            prop_assert_eq!(got, want, "{}: list activity", at);
        }
    }
    Ok(())
}

async fn run_case(env: &Env, ops: &[Op]) -> Result<(), TestCaseError> {
    let fix = Fix::new();
    let sys = SystemScope::mint();
    bounded("seed", env.db.seed(&sys, &fix.seed())).await?;
    let mut m = Model::new(fix);
    let mut tx: Option<Box<dyn Tx>> = None;
    for call in m.setup() {
        execute(env.db.as_ref(), &mut tx, call, "seed the case's logins, sessions, keys and links").await?;
    }
    // A failed earlier case (shrinking runs many) may have left rows behind: take them out of the way first.
    let at = tick();
    for row in bounded("start", env.db.undispatched_outbox(&sys, 10_000)).await? {
        bounded("start", env.db.mark_outbox_dispatched(&sys, row.envelope.event_id, at)).await?;
    }
    execute(env.db.as_ref(), &mut tx, Call::Depth { want: 0 }, "start: no undispatched rows left").await?;
    for (step, op) in ops.iter().enumerate() {
        let (calls, classes) = m.plan(op);
        for (n, call) in calls.into_iter().enumerate() {
            let at = format!("step {step}: {op:?} {classes:?}, call {n}: {call:?}");
            execute(env.db.as_ref(), &mut tx, call, &at).await?;
        }
    }
    for call in m.drain() {
        let at = format!("end of case: {call:?}");
        execute(env.db.as_ref(), &mut tx, call, &at).await?;
    }
    Ok(())
}

/// Proptest config: the case budget, a bounded shrink (each step is a run against Postgres), and counterexamples
/// saved beside this file (`mod.proptest-regressions`, committed per Rule 7).
fn config() -> Config {
    let cases = std::env::var("THRUST_MODEL_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(32);
    let failure_persistence: Option<Box<dyn FailurePersistence>> =
        if std::env::var_os("PROPTEST_DISABLE_FAILURE_PERSISTENCE").is_some() {
            None
        } else {
            Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions")))
        };
    Config {
        cases,
        failure_persistence,
        source_file: Some(file!()),
        max_shrink_iters: 512,
        max_shrink_time: 120_000,
        ..Config::default()
    }
}

#[test]
fn model_postgres_adapter_matches_the_reference_model() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let env = rt.block_on(env());
    let config = config();
    assert!(config.cases > 0, "THRUST_MODEL_CASES=0 would check nothing");
    let mut runner = TestRunner::new(config);
    let result = runner.run(&ops(), |ops| rt.block_on(run_case(&env, &ops)));
    rt.block_on(async move { drop(env) });
    if let Err(e) = result {
        panic!("{e}");
    }
}

/// Rule 8 for the test above: samples sequences with a fixed seed, plans them with the model alone and requires every
/// step class at least once (a generator that never deletes a row inside its own transaction, say, would pass on an
/// adapter that gets that wrong).
#[test]
fn model_postgres_strategy_reaches_every_step_class() {
    let mut runner = TestRunner::deterministic();
    let mut seen = BTreeSet::new();
    // 3000, not 1000: every op added to the strategy thins out the others, and a rare class (`GotDeletedHidden`) was one
    // sample from missing at 1000. The control is stronger for the larger sample, and planning is model-only and quick.
    for _ in 0..3000 {
        let ops = ops().new_tree(&mut runner).unwrap().current();
        let mut m = Model::new(Fix::new());
        for op in &ops {
            seen.extend(m.plan(op).1);
        }
    }
    let missing: Vec<_> = Class::ALL.iter().filter(|c| !seen.contains(c)).collect();
    assert!(missing.is_empty(), "never generated: {missing:?}");
}
