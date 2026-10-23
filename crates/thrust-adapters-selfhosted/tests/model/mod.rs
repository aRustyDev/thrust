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
//!   number; a transaction that commits keeps them, one that rolls back or is dropped unfinished gives them back (the
//!   counter row is transactional in Postgres: no gap, unlike the memory adapter); `allocate` under another
//!   workspace than the project's is refused, and the transaction is then rolled back;
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

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (prop::collection::vec(step(), 1..5), end()).prop_map(|(steps, end)| Op::Tx { steps, end }),
        2 => (proj(), ws(), any::<bool>()).prop_map(|(proj, ws, with_deleted)| Op::List { proj, ws, with_deleted }),
        2 => (proj(), ws(), prop::option::weighted(0.85, 0..16usize), any::<bool>())
            .prop_map(|(proj, ws, pick, with_deleted)| Op::Get { proj, ws, pick, with_deleted }),
        2 => (proj(), sl(), user()).prop_map(|(proj, slug, user)| Op::Membership { proj, slug, user }),
        2 => (0..3usize, when()).prop_map(|(sess, when)| Op::SessionLookup { sess, when }),
        1 => (0..3usize).prop_map(|sess| Op::SessionTouch { sess }),
        1 => (0..3usize).prop_map(|sess| Op::SessionRevoke { sess }),
        1 => (0..2usize).prop_map(|sess| Op::SessionReinsert { sess }),
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
    const ALL: [Class; 47] = [
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
    Undispatched { limit: u32, want: Vec<OutboxRow> },
    Mark { event: EventId, at: Timestamp },
    Depth { want: u64 },
    InsertActivity { a: Activity, want: bool },
    ListActivity { ws: WorkspaceId, p: ProjectId, entity: Option<WorkItemId>, want: Vec<Activity> },
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
        }
    }

    fn user(&self, u: U) -> UserId {
        self.users[u as usize]
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

/// The spec's membership answer: only the project's own slug resolves it, and only for a member of that workspace.
fn membership(fix: &Fix, proj: P, sl: Sl, user: U) -> Membership {
    let s = slot(proj);
    if sl != Sl::Own || !in_workspace(user)[s] {
        return Membership::NoSuchProject;
    }
    match role_in(proj, user) {
        Some(role) => Membership::Member { workspace_id: fix.ws[s], role },
        None => Membership::NotAMember,
    }
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
        Model {
            fix,
            sessions,
            rows: [Vec::new(), Vec::new()],
            counter: [0, 0],
            outbox: Vec::new(),
            activity: Vec::new(),
            projected: BTreeSet::new(),
        }
    }

    /// The calls that create the case's two sessions; their answers are all `()`.
    fn setup(&self) -> Vec<Call> {
        self.sessions.iter().map(|s| Call::InsertSession { s: s.new.clone() }).collect()
    }

    fn undispatched(&self) -> Vec<OutboxRow> {
        let mut rows: Vec<OutboxRow> = self.outbox.iter().filter(|(_, d)| !d).map(|(r, _)| r.clone()).collect();
        rows.sort_by_key(|r| (r.created_at, r.envelope.event_id));
        rows
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
                let want = membership(&self.fix, proj, slug, user);
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
                        let live = !m.revoked && m.new.expires_at > now;
                        let class = match (m.revoked, live) {
                            (true, _) => Class::SessionRevokedHidden,
                            (false, false) => Class::SessionExpiredHidden,
                            (false, true) => Class::SessionLive,
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
        if failed {
            classes.push(Class::FailedAndRolledBack);
            calls.push(Call::Rollback);
            return;
        }
        match end {
            End::Commit => {
                classes.push(Class::Committed);
                calls.push(Call::Commit);
                for s in 0..2 {
                    self.counter[s] += allocs[s];
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
                calls.push(Call::Rollback);
            }
            End::Abandon => {
                classes.push(Class::Abandoned);
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
        execute(env.db.as_ref(), &mut tx, call, "seed the case's sessions").await?;
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
    for _ in 0..1000 {
        let ops = ops().new_tree(&mut runner).unwrap().current();
        let mut m = Model::new(Fix::new());
        for op in &ops {
            seen.extend(m.plan(op).1);
        }
    }
    let missing: Vec<_> = Class::ALL.iter().filter(|c| !seen.contains(c)).collect();
    assert!(missing.is_empty(), "never generated: {missing:?}");
}
