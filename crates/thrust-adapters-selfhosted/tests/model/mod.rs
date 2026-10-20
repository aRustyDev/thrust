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
//! * `membership` is `NoSuchProject` for a project outside the workspace asked about, `NotAMember` without a
//!   membership, and the member's role otherwise;
//! * inside a transaction, `allocate` hands out the project's next sequence number and `sort_order` = 10 000 × that
//!   number; a transaction that commits keeps them, one that rolls back or is dropped unfinished gives them back (the
//!   counter row is transactional in Postgres: no gap, unlike the memory adapter);
//! * `insert_work_item` is a `Conflict` when the project already has a live row with the same external key, rows of
//!   the same transaction included; the transaction is then rolled back, as the core does;
//! * `soft_delete_work_item` is `true` exactly for a live row of that workspace and project (one inserted earlier in
//!   the same transaction included) and `false` for a deleted row, an unknown id or the wrong workspace; a soft
//!   delete frees the external key;
//! * nothing a transaction wrote (rows, deletes, outbox rows) is visible after a rollback or a drop, and all of it is
//!   after a commit; a dropped transaction neither holds locks nor poisons the pool (`mutant_discard_noop`);
//! * `list_work_items` returns exactly the project's rows in sequence order (live ones, or all with `WithDeleted`),
//!   and nothing through another workspace; `get_work_item` likewise for one id;
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
use thrust_contracts::chrono::{TimeZone, Utc};
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

#[derive(Debug, Clone, Copy)]
enum U {
    AdminA,
    MemberA,
    GuestA,
    /// In workspace A, member of nothing.
    OutsiderA,
    AdminB,
}

#[derive(Debug, Clone, Copy)]
enum Step {
    /// `ext`: an external key from a small domain, so that keys collide.
    Create { proj: P, ext: Option<u8> },
    /// `pick`: one of the project's rows, committed or written earlier in this transaction (an unknown id if none).
    Delete { proj: P, ws: Ws, pick: usize },
    Outbox { proj: P },
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
    Membership { proj: P, ws: Ws, user: U },
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

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (proj(), prop::option::weighted(0.5, 0..3u8)).prop_map(|(proj, ext)| Step::Create { proj, ext }),
        2 => (proj(), ws(), 0..16usize).prop_map(|(proj, ws, pick)| Step::Delete { proj, ws, pick }),
        2 => proj().prop_map(|proj| Step::Outbox { proj }),
    ]
}

fn end() -> impl Strategy<Value = End> {
    prop_oneof![5 => Just(End::Commit), 2 => Just(End::Rollback), 2 => Just(End::Abandon)]
}

fn user() -> impl Strategy<Value = U> {
    prop_oneof![Just(U::AdminA), Just(U::MemberA), Just(U::GuestA), Just(U::OutsiderA), Just(U::AdminB)]
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
        1 => (proj(), ws(), user()).prop_map(|(proj, ws, user)| Op::Membership { proj, ws, user }),
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
    NoSuchProject,
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
}

impl Class {
    const ALL: [Class; 35] = [
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
    ];
}

/// One port call and the answer the adapter must give.
#[derive(Debug, Clone)]
enum Call {
    Begin,
    Allocate { ws: WorkspaceId, p: ProjectId, want: Allocation },
    Insert { item: WorkItem, conflict: bool },
    SoftDelete { ws: WorkspaceId, p: ProjectId, id: WorkItemId, at: Timestamp, want: bool },
    InsertOutbox { row: OutboxRow },
    Commit,
    Rollback,
    Abandon,
    List { ws: WorkspaceId, p: ProjectId, vis: Visibility, want: Vec<WorkItem> },
    Get { ws: WorkspaceId, p: ProjectId, id: WorkItemId, vis: Visibility, want: Option<WorkItem> },
    Membership { ws: WorkspaceId, p: ProjectId, user: UserId, want: Membership },
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
    p: [ProjectId; 2],
    /// AdminA, MemberA, GuestA, OutsiderA, AdminB.
    users: [UserId; 5],
}

impl Fix {
    fn new() -> Fix {
        Fix {
            ws: [WorkspaceId(new_id()), WorkspaceId(new_id())],
            p: [ProjectId(new_id()), ProjectId(new_id())],
            users: [UserId(new_id()), UserId(new_id()), UserId(new_id()), UserId(new_id()), UserId(new_id())],
        }
    }

    fn seed(&self) -> SeedFixture {
        let [admin_a, member_a, guest_a, _outsider_a, admin_b] = self.users;
        let m = |s: usize, user_id, role| SeedMembership { project_id: self.p[s], workspace_id: self.ws[s], user_id, role };
        SeedFixture {
            workspaces: self.ws.to_vec(),
            users: self.users.to_vec(),
            projects: vec![
                SeedProject { id: self.p[0], workspace_id: self.ws[0], name: "A".into() },
                SeedProject { id: self.p[1], workspace_id: self.ws[1], name: "B".into() },
            ],
            memberships: vec![
                m(0, admin_a, Role::Admin),
                m(0, member_a, Role::Member),
                m(0, guest_a, Role::Guest),
                m(1, admin_b, Role::Admin),
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
}

fn slot(p: P) -> usize {
    match p {
        P::A => 0,
        P::B => 1,
    }
}

/// The spec's membership answer, written out from `Fix::seed`.
fn membership(proj: P, ws: Ws, user: U) -> Membership {
    match (ws, proj, user) {
        (Ws::Other, _, _) => Membership::NoSuchProject,
        (Ws::Own, P::A, U::AdminA) => Membership::Member(Role::Admin),
        (Ws::Own, P::A, U::MemberA) => Membership::Member(Role::Member),
        (Ws::Own, P::A, U::GuestA) => Membership::Member(Role::Guest),
        (Ws::Own, P::A, U::OutsiderA | U::AdminB) => Membership::NotAMember,
        (Ws::Own, P::B, U::AdminB) => Membership::Member(Role::Admin),
        (Ws::Own, P::B, U::AdminA | U::MemberA | U::GuestA | U::OutsiderA) => Membership::NotAMember,
    }
}

#[derive(Debug)]
struct Model {
    fix: Fix,
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
        Model {
            fix,
            rows: [Vec::new(), Vec::new()],
            counter: [0, 0],
            outbox: Vec::new(),
            activity: Vec::new(),
            projected: BTreeSet::new(),
        }
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
            &Op::Membership { proj, ws, user } => {
                let want = membership(proj, ws, user);
                classes.push(match want {
                    Membership::Member(_) => Class::IsMember,
                    Membership::NotAMember => Class::NotAMember,
                    Membership::NoSuchProject => Class::NoSuchProject,
                });
                let (w, p, u) = (self.fix.ws_for(proj, ws), self.fix.p[slot(proj)], self.fix.user(user));
                calls.push(Call::Membership { ws: w, p, user: u, want });
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
        calls.push(Call::Begin);
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
                    calls.push(Call::Allocate { ws: self.fix.ws[s], p: self.fix.p[s], want: a });
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

fn open<'a>(tx: &'a mut Option<Box<dyn Tx>>, at: &str) -> Result<&'a mut (dyn Tx + 'static), TestCaseError> {
    tx.as_deref_mut().ok_or_else(|| fail(at, "no open transaction"))
}

async fn execute(db: &dyn Database, tx: &mut Option<Box<dyn Tx>>, call: Call, at: &str) -> Result<(), TestCaseError> {
    let open = |tx| open(tx, at);
    match call {
        Call::Begin => *tx = Some(bounded(at, db.begin()).await?),
        Call::Allocate { ws, p, want } => {
            let got = bounded(at, open(tx)?.allocate(ws, p)).await?;
            prop_assert_eq!(got, want, "{}: allocation", at);
        }
        Call::Insert { item, conflict } => {
            let got = tokio::time::timeout(Duration::from_secs(20), open(tx)?.insert_work_item(&item))
                .await
                .map_err(|_| fail(at, "timed out after 20 s"))?;
            match (got, conflict) {
                (Ok(()), false) | (Err(DbError::Conflict(_)), true) => {}
                (got, _) => return Err(fail(at, format!("insert: got {got:?}, want conflict = {conflict}"))),
            }
        }
        Call::SoftDelete { ws, p, id, at: when, want } => {
            let got = bounded(at, open(tx)?.soft_delete_work_item(ws, p, id, when)).await?;
            prop_assert_eq!(got, want, "{}: soft delete", at);
        }
        Call::InsertOutbox { row } => bounded(at, open(tx)?.insert_outbox(&row)).await?,
        Call::Commit => bounded(at, tx.take().ok_or_else(|| fail(at, "no open transaction"))?.commit()).await?,
        Call::Rollback => bounded(at, tx.take().ok_or_else(|| fail(at, "no open transaction"))?.rollback()).await?,
        Call::Abandon => drop(tx.take()),
        Call::List { ws, p, vis, want } => {
            let got = bounded(at, db.list_work_items(ws, p, vis)).await?;
            prop_assert_eq!(got, want, "{}: list", at);
        }
        Call::Get { ws, p, id, vis, want } => {
            let got = bounded(at, db.get_work_item(ws, p, id, vis)).await?;
            prop_assert_eq!(got, want, "{}: get", at);
        }
        Call::Membership { ws, p, user, want } => {
            let got = bounded(at, db.membership(ws, p, user)).await?;
            prop_assert_eq!(got, want, "{}: membership", at);
        }
        Call::Undispatched { limit, want } => {
            let got = bounded(at, db.undispatched_outbox(limit)).await?;
            prop_assert_eq!(got, want, "{}: undispatched outbox", at);
        }
        Call::Mark { event, at: when } => bounded(at, db.mark_outbox_dispatched(event, when)).await?,
        Call::Depth { want } => {
            let got = bounded(at, db.outbox_depth()).await?;
            prop_assert_eq!(got, want, "{}: outbox depth", at);
        }
        Call::InsertActivity { a, want } => {
            let got = bounded(at, db.insert_activity(&a)).await?;
            prop_assert_eq!(got, want, "{}: insert activity (true = new event id)", at);
        }
        Call::ListActivity { ws, p, entity, want } => {
            let got = bounded(at, db.list_activity(ws, p, entity)).await?;
            prop_assert_eq!(got, want, "{}: list activity", at);
        }
    }
    Ok(())
}

async fn run_case(env: &Env, ops: &[Op]) -> Result<(), TestCaseError> {
    let fix = Fix::new();
    bounded("seed", env.db.seed(&fix.seed())).await?;
    let mut m = Model::new(fix);
    let mut tx: Option<Box<dyn Tx>> = None;
    // A failed earlier case (shrinking runs many) may have left rows behind: take them out of the way first.
    let at = tick();
    for row in bounded("start", env.db.undispatched_outbox(10_000)).await? {
        bounded("start", env.db.mark_outbox_dispatched(row.envelope.event_id, at)).await?;
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
