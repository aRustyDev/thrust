//! Generated scenarios over HTTP (gap G-8 in `docs/src/testing/status.md`). Part of the portable suite: the tests
//! here run in the `main` pass of `scripts/contract.sh` on every target, through the same `THRUST_BASE_URL` client.
//!
//! * `model_work_items_over_http_match_the_reference_model`: random sequences of create, delete and list, by every
//!   kind of caller, in two tenants and a project that does not exist, checked against a reference model after
//!   every step.
//! * `every_actor_project_and_route_gets_the_status_of_the_access_table`: every caller × project × route, once each.
//! * `model_strategy_reaches_every_step_class`: the generator coverage control for the first test (Rule 8). It
//!   replays sampled sequences through the model alone and needs no target.
//!
//! The model is the spec, written out; it shares no code with the server:
//!
//! * a missing or unknown bearer token is `401` before anything else;
//! * a create with a bad name (blank, or over 255 characters after trimming) is `400` before the rate limiter and
//!   authorization, and a list with an unknown `scope` is `400` before authorization;
//! * a project outside the caller's workspace, or one that does not exist, is `404`; a project the caller is not a
//!   member of is `403`; create and delete need Member, list and activity need Guest, `scope=with_deleted` needs
//!   Admin;
//! * sequence ids per project start at 1, strictly increase and are never reused. A create that fails after the
//!   number was allocated (a `409`) may leave a gap, at most one per failed create: ADR-0043 accepts gaps on
//!   failure, and the adapters differ here (memory leaves the gap, Postgres rolls it back);
//! * a live `(external_source, external_id)` is unique per project (`409`), and a soft delete frees it;
//! * a delete is `204` for a live item of the path's project and `404` for a deleted item, an unknown id or another
//!   project's item; a rejected request changes nothing (every later list checks that);
//! * a list returns exactly the model's rows of that project, in sequence order, trimmed names, `deleted_at` set
//!   exactly on deleted rows, and nothing outside the requested project (R-S1).
//!
//! Budget: `THRUST_MODEL_CASES` sequences per run (default 16), deliberately not `PROPTEST_CASES`, so a `deep` run's
//! setting cannot turn one contract cell into thousands of round trips. Each case seeds its own fixture, so it starts
//! from empty projects and fresh rate-limit buckets; `MAX_OPS` stays below the create limit so the limiter is never
//! reached (`create_is_rate_limited_per_user` owns that rule).

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, FailurePersistence, FileFailurePersistence, TestCaseError, TestRunner};
use reqwest::Method;
use serde_json::{json, Value};
use thrust_contract_tests::*;
use thrust_contracts::*;
use uuid::Uuid;

const MAX_OPS: usize = 24;

// ---------------------------------------------------------------------------------------------
// The spec: who may do what
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Actor {
    AdminA,
    MemberA,
    GuestA,
    /// In workspace A, member of no project.
    OutsiderA,
    AdminB,
    /// No `Authorization` header.
    Anonymous,
    /// A bearer token that is not a session.
    BadToken,
}

/// Every actor. A new variant fails to compile in `standing` until the table says what it may do; add it here too.
const ALL_ACTORS: [Actor; 7] =
    [Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA, Actor::AdminB, Actor::Anonymous, Actor::BadToken];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Proj {
    A,
    B,
    /// A project id that was never seeded.
    Unknown,
}

const ALL_PROJECTS: [Proj; 3] = [Proj::A, Proj::B, Proj::Unknown];

enum Standing {
    Unauthenticated,
    NoSuchProject,
    NotAMember,
    Member(Role),
}

/// What `actor` is in `proj`, from the fixture's memberships (`Fixture::seed`), written out rather than asked of
/// the server.
fn standing(actor: Actor, proj: Proj) -> Standing {
    match (actor, proj) {
        (Actor::Anonymous | Actor::BadToken, _) => Standing::Unauthenticated,
        (Actor::AdminA, Proj::A) => Standing::Member(Role::Admin),
        (Actor::MemberA, Proj::A) => Standing::Member(Role::Member),
        (Actor::GuestA, Proj::A) => Standing::Member(Role::Guest),
        (Actor::OutsiderA, Proj::A) => Standing::NotAMember,
        (Actor::AdminB, Proj::B) => Standing::Member(Role::Admin),
        (Actor::AdminA | Actor::MemberA | Actor::GuestA | Actor::OutsiderA, Proj::B) => Standing::NoSuchProject,
        (Actor::AdminB, Proj::A) => Standing::NoSuchProject,
        (Actor::AdminA | Actor::MemberA | Actor::GuestA | Actor::OutsiderA | Actor::AdminB, Proj::Unknown) => {
            Standing::NoSuchProject
        }
    }
}

/// The rejection a request needing `min` gets from authentication and authorization, or `None` when it passes.
fn gate(actor: Actor, proj: Proj, min: Role) -> Option<(u16, Class)> {
    match standing(actor, proj) {
        Standing::Unauthenticated => Some((401, Class::Unauthenticated)),
        Standing::NoSuchProject if proj == Proj::Unknown => Some((404, Class::NoSuchProject)),
        Standing::NoSuchProject => Some((404, Class::CrossTenant)),
        Standing::NotAMember => Some((403, Class::NotAMember)),
        Standing::Member(r) if r >= min => None,
        Standing::Member(_) => Some((403, Class::RoleTooLow)),
    }
}

// ---------------------------------------------------------------------------------------------
// Operations and their generators
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Name {
    Plain,
    /// Leading and trailing spaces, stored trimmed.
    Padded,
    /// Exactly 255 characters, the longest valid name.
    Longest,
    Blank,
    /// 256 characters.
    TooLong,
}

#[derive(Debug, Clone, Copy)]
enum Scope {
    Default,
    Visible,
    WithDeleted,
    Bogus,
}

#[derive(Debug, Clone, Copy)]
enum Pick {
    /// One of the path project's items, live or deleted (an unknown id when it has none).
    Own(usize),
    /// One of the other seeded project's items: a delete must not reach it through this path.
    OtherProject(usize),
    Unknown,
}

#[derive(Debug, Clone)]
enum Op {
    /// `ext`: an external key from a small domain, so that keys collide.
    Create { actor: Actor, proj: Proj, name: Name, ext: Option<u8> },
    Delete { actor: Actor, proj: Proj, pick: Pick },
    List { actor: Actor, proj: Proj, scope: Scope },
}

fn actor() -> impl Strategy<Value = Actor> {
    // Weighted towards the callers who can write, so sequences build up state worth checking.
    prop_oneof![
        3 => Just(Actor::AdminA),
        4 => Just(Actor::MemberA),
        1 => Just(Actor::GuestA),
        1 => Just(Actor::OutsiderA),
        3 => Just(Actor::AdminB),
        1 => Just(Actor::Anonymous),
        1 => Just(Actor::BadToken),
    ]
}

fn proj() -> impl Strategy<Value = Proj> {
    prop_oneof![5 => Just(Proj::A), 3 => Just(Proj::B), 1 => Just(Proj::Unknown)]
}

fn name() -> impl Strategy<Value = Name> {
    prop_oneof![
        6 => Just(Name::Plain),
        1 => Just(Name::Padded),
        1 => Just(Name::Longest),
        1 => Just(Name::Blank),
        1 => Just(Name::TooLong),
    ]
}

fn scope() -> impl Strategy<Value = Scope> {
    prop_oneof![3 => Just(Scope::Default), 1 => Just(Scope::Visible), 3 => Just(Scope::WithDeleted), 1 => Just(Scope::Bogus)]
}

fn pick() -> impl Strategy<Value = Pick> {
    prop_oneof![6 => (0..16usize).prop_map(Pick::Own), 2 => (0..16usize).prop_map(Pick::OtherProject), 1 => Just(Pick::Unknown)]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        5 => (actor(), proj(), name(), prop::option::weighted(0.4, 0..3u8))
            .prop_map(|(actor, proj, name, ext)| Op::Create { actor, proj, name, ext }),
        3 => (actor(), proj(), pick()).prop_map(|(actor, proj, pick)| Op::Delete { actor, proj, pick }),
        3 => (actor(), proj(), scope()).prop_map(|(actor, proj, scope)| Op::List { actor, proj, scope }),
    ]
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(op(), 1..=MAX_OPS)
}

// ---------------------------------------------------------------------------------------------
// The reference model
// ---------------------------------------------------------------------------------------------

/// What kind of step the model predicted. The generator coverage control requires every one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Unauthenticated,
    InvalidName,
    BogusScope,
    CrossTenant,
    NoSuchProject,
    NotAMember,
    RoleTooLow,
    Created,
    /// A create after a `409` in the same project: the place where a sequence gap may appear.
    CreatedAfterFailure,
    /// A create reusing the external key of a soft-deleted row.
    ExternalKeyFreedByDelete,
    Conflict,
    Deleted,
    DeleteAlreadyDeleted,
    DeleteOtherProjectsItem,
    DeleteUnknownId,
    Listed,
    ListedWithDeleted,
}

impl Class {
    const ALL: [Class; 17] = [
        Class::Unauthenticated,
        Class::InvalidName,
        Class::BogusScope,
        Class::CrossTenant,
        Class::NoSuchProject,
        Class::NotAMember,
        Class::RoleTooLow,
        Class::Created,
        Class::CreatedAfterFailure,
        Class::ExternalKeyFreedByDelete,
        Class::Conflict,
        Class::Deleted,
        Class::DeleteAlreadyDeleted,
        Class::DeleteOtherProjectsItem,
        Class::DeleteUnknownId,
        Class::Listed,
        Class::ListedWithDeleted,
    ];
}

#[derive(Debug, Clone)]
struct Row {
    id: WorkItemId,
    seq: i64,
    name: String,
    deleted: bool,
}

#[derive(Debug, Clone, Default)]
struct ProjectModel {
    rows: Vec<Row>,
    /// Live external keys and the keys of deleted rows, by key.
    live_ext: BTreeSet<u8>,
    deleted_ext: BTreeSet<u8>,
    last_seq: i64,
    last_sort: i64,
    /// `409`s since the last successful create: each may have used up a sequence number.
    failed_since: i64,
}

#[derive(Debug, Default)]
struct Model {
    projects: [ProjectModel; 2],
}

fn slot(p: Proj) -> Option<usize> {
    match p {
        Proj::A => Some(0),
        Proj::B => Some(1),
        Proj::Unknown => None,
    }
}

#[derive(Debug, Clone)]
enum Want {
    /// A rejection, checked on its status.
    Reject(u16),
    Conflict { slot: usize, ext: u8 },
    Create { slot: usize, name: String, ext: Option<u8> },
    Delete { slot: usize, row: usize },
    List { slot: usize, with_deleted: bool },
}

/// One HTTP request, in model terms; `World::send` turns it into ids.
#[derive(Debug, Clone)]
struct Req {
    actor: Actor,
    method: Method,
    proj: Proj,
    item: Option<WorkItemId>,
    query: Option<&'static str>,
    body: Option<Value>,
}

struct Plan {
    req: Req,
    want: Want,
    class: Class,
}

fn unknown_item(step: usize) -> WorkItemId {
    WorkItemId(Uuid::from_u128(0xBAD0_0000 + step as u128))
}

impl Model {
    fn plan(&self, step: usize, op: &Op) -> Plan {
        match *op {
            Op::Create { actor, proj, name, ext } => {
                let raw = match name {
                    Name::Plain => format!("item {step}"),
                    Name::Padded => format!("  item {step}  "),
                    Name::Longest => format!("{step:03}{}", "y".repeat(252)),
                    Name::Blank => "   ".to_string(),
                    Name::TooLong => "x".repeat(256),
                };
                let mut body = json!({ "name": raw });
                if let Some(k) = ext {
                    body["external_source"] = json!("gh");
                    body["external_id"] = json!(format!("k{k}"));
                }
                let req = Req { actor, method: Method::POST, proj, item: None, query: None, body: Some(body) };
                let (want, class) = if matches!(standing(actor, proj), Standing::Unauthenticated) {
                    (Want::Reject(401), Class::Unauthenticated)
                } else if matches!(name, Name::Blank | Name::TooLong) {
                    (Want::Reject(400), Class::InvalidName)
                } else if let Some((status, class)) = gate(actor, proj, Role::Member) {
                    (Want::Reject(status), class)
                } else {
                    let s = slot(proj).expect("an authorized project is a seeded one");
                    let p = &self.projects[s];
                    match ext {
                        Some(k) if p.live_ext.contains(&k) => (Want::Conflict { slot: s, ext: k }, Class::Conflict),
                        _ => {
                            let class = if p.failed_since > 0 {
                                Class::CreatedAfterFailure
                            } else if ext.is_some_and(|k| p.deleted_ext.contains(&k)) {
                                Class::ExternalKeyFreedByDelete
                            } else {
                                Class::Created
                            };
                            (Want::Create { slot: s, name: raw.trim().to_string(), ext }, class)
                        }
                    }
                };
                Plan { req, want, class }
            }
            Op::Delete { actor, proj, pick } => {
                let own = slot(proj);
                let other = match proj {
                    Proj::A | Proj::Unknown => 1,
                    Proj::B => 0,
                };
                let rows_of = |s: usize| &self.projects[s].rows;
                // (item id, the own-project row it is, if any)
                let (item, row) = match pick {
                    Pick::Own(i) => match own {
                        Some(s) if !rows_of(s).is_empty() => {
                            let r = i % rows_of(s).len();
                            (rows_of(s)[r].id, Some(r))
                        }
                        _ => (unknown_item(step), None),
                    },
                    Pick::OtherProject(i) if !rows_of(other).is_empty() => {
                        (rows_of(other)[i % rows_of(other).len()].id, None)
                    }
                    Pick::OtherProject(_) | Pick::Unknown => (unknown_item(step), None),
                };
                let req = Req { actor, method: Method::DELETE, proj, item: Some(item), query: None, body: None };
                let (want, class) = if let Some((status, class)) = gate(actor, proj, Role::Member) {
                    (Want::Reject(status), class)
                } else {
                    let s = own.expect("an authorized project is a seeded one");
                    match (pick, row) {
                        (_, Some(r)) if !rows_of(s)[r].deleted => (Want::Delete { slot: s, row: r }, Class::Deleted),
                        (_, Some(_)) => (Want::Reject(404), Class::DeleteAlreadyDeleted),
                        (Pick::OtherProject(_), None) if !rows_of(other).is_empty() => {
                            (Want::Reject(404), Class::DeleteOtherProjectsItem)
                        }
                        _ => (Want::Reject(404), Class::DeleteUnknownId),
                    }
                };
                Plan { req, want, class }
            }
            Op::List { actor, proj, scope } => {
                let query = match scope {
                    Scope::Default => None,
                    Scope::Visible => Some("visible"),
                    Scope::WithDeleted => Some("with_deleted"),
                    Scope::Bogus => Some("bogus"),
                };
                let req = Req { actor, method: Method::GET, proj, item: None, query, body: None };
                let with_deleted = matches!(scope, Scope::WithDeleted);
                let min = if with_deleted { Role::Admin } else { Role::Guest };
                let (want, class) = if matches!(standing(actor, proj), Standing::Unauthenticated) {
                    (Want::Reject(401), Class::Unauthenticated)
                } else if matches!(scope, Scope::Bogus) {
                    (Want::Reject(400), Class::BogusScope)
                } else if let Some((status, class)) = gate(actor, proj, min) {
                    (Want::Reject(status), class)
                } else {
                    let s = slot(proj).expect("an authorized project is a seeded one");
                    let class = if with_deleted { Class::ListedWithDeleted } else { Class::Listed };
                    (Want::List { slot: s, with_deleted }, class)
                };
                Plan { req, want, class }
            }
        }
    }

    fn record_conflict(&mut self, slot: usize) {
        self.projects[slot].failed_since += 1;
    }

    fn record_create(&mut self, slot: usize, row: Row, sort: i64, ext: Option<u8>) {
        let p = &mut self.projects[slot];
        p.last_seq = row.seq;
        p.last_sort = sort;
        p.failed_since = 0;
        if let Some(k) = ext {
            p.live_ext.insert(k);
            p.deleted_ext.remove(&k);
        }
        p.rows.push(row);
    }

    fn record_delete(&mut self, slot: usize, row: usize, ext_of: impl Fn(&Row) -> Option<u8>) {
        let p = &mut self.projects[slot];
        p.rows[row].deleted = true;
        if let Some(k) = ext_of(&p.rows[row]) {
            p.live_ext.remove(&k);
            p.deleted_ext.insert(k);
        }
    }

    /// The rows a list of `slot` must return: (sequence id, name, deleted).
    fn expected_rows(&self, slot: usize, with_deleted: bool) -> Vec<(i64, String, bool)> {
        self.projects[slot]
            .rows
            .iter()
            .filter(|r| with_deleted || !r.deleted)
            .map(|r| (r.seq, r.name.clone(), r.deleted))
            .collect()
    }

    /// Offline replay for the coverage control: apply the predicted effect with made-up ids.
    fn simulate(&mut self, step: usize, plan: &Plan, exts: &mut Vec<(WorkItemId, u8)>) {
        match plan.want {
            Want::Reject(_) | Want::List { .. } => {}
            Want::Conflict { slot, .. } => self.record_conflict(slot),
            Want::Create { slot, ref name, ext } => {
                let id = WorkItemId(Uuid::from_u128(step as u128 + 1));
                if let Some(k) = ext {
                    exts.push((id, k));
                }
                let seq = self.projects[slot].last_seq + 1;
                self.record_create(slot, Row { id, seq, name: name.clone(), deleted: false }, seq * 10_000, ext);
            }
            Want::Delete { slot, row } => {
                self.record_delete(slot, row, |r| exts.iter().find(|(id, _)| *id == r.id).map(|(_, k)| *k))
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The HTTP side
// ---------------------------------------------------------------------------------------------

struct World<'a> {
    t: &'a Target,
    f: Fixture,
    unknown: ProjectId,
    /// External key of each created row, by id (the row's `external_id` as the model chose it).
    exts: Vec<(WorkItemId, u8)>,
}

impl<'a> World<'a> {
    async fn seed(t: &'a Target) -> World<'a> {
        let f = Fixture::seed(t).await;
        World { t, f, unknown: ProjectId(Uuid::new_v4()), exts: Vec::new() }
    }

    fn user(&self, a: Actor) -> Option<&User> {
        match a {
            Actor::AdminA => Some(&self.f.admin_a),
            Actor::MemberA => Some(&self.f.member_a),
            Actor::GuestA => Some(&self.f.guest_a),
            Actor::OutsiderA => Some(&self.f.outsider_a),
            Actor::AdminB => Some(&self.f.admin_b),
            Actor::Anonymous | Actor::BadToken => None,
        }
    }

    fn project(&self, p: Proj) -> ProjectId {
        match p {
            Proj::A => self.f.project_a,
            Proj::B => self.f.project_b,
            Proj::Unknown => self.unknown,
        }
    }

    fn scope_of(&self, slot: usize) -> (WorkspaceId, ProjectId) {
        if slot == 0 {
            (self.f.ws_a, self.f.project_a)
        } else {
            (self.f.ws_b, self.f.project_b)
        }
    }

    fn as_actor(&self, b: reqwest::RequestBuilder, actor: Actor) -> reqwest::RequestBuilder {
        match actor {
            Actor::Anonymous => b,
            Actor::BadToken => b.bearer_auth("not-a-session"),
            a => b.bearer_auth(&self.user(a).expect("an authenticated actor has a user").token),
        }
    }

    async fn send(&self, req: &Req) -> (u16, String) {
        let mut path = wi_path(self.project(req.proj));
        if let Some(id) = req.item {
            path.push_str(&format!("/{id}"));
        }
        if let Some(q) = req.query {
            path.push_str(&format!("?scope={q}"));
        }
        let mut b = self.as_actor(self.t.http.request(req.method.clone(), self.t.url(&path)), req.actor);
        if let Some(body) = &req.body {
            b = b.json(body);
        }
        let r = b.send().await.unwrap();
        let status = r.status().as_u16();
        (status, r.text().await.unwrap_or_default())
    }
}

/// Sequences run and requests sent, shrinking included: the test fails if it compared nothing. The counts are printed
/// as `MODEL-OBSERVATION` (visible on failure, or with `--nocapture`).
static CASES: AtomicUsize = AtomicUsize::new(0);
static REQUESTS: AtomicUsize = AtomicUsize::new(0);

async fn run_case(t: &Target, ops: &[Op]) -> Result<(), TestCaseError> {
    CASES.fetch_add(1, Ordering::Relaxed);
    let mut w = World::seed(t).await;
    let mut m = Model::default();
    for (step, op) in ops.iter().enumerate() {
        let plan = m.plan(step, op);
        let (status, body) = w.send(&plan.req).await;
        REQUESTS.fetch_add(1, Ordering::Relaxed);
        let at = format!("step {step}: {op:?} ({:?})", plan.class);
        match plan.want {
            Want::Reject(want) => prop_assert_eq!(status, want, "{}; body {}", at, body),
            Want::Conflict { slot, ext } => {
                prop_assert_eq!(status, 409, "{}: external key k{} is live; body {}", at, ext, body);
                m.record_conflict(slot);
            }
            Want::Create { slot, ref name, ext } => {
                prop_assert_eq!(status, 201, "{}; body {}", at, body);
                let item: WorkItem = serde_json::from_str(&body).map_err(|e| TestCaseError::fail(format!("{at}: {e}")))?;
                let p = &m.projects[slot];
                prop_assert_eq!((item.workspace_id, item.project_id), w.scope_of(slot), "{}: created outside its scope", at);
                prop_assert_eq!(&item.name, name, "{}: the stored name is the trimmed name", at);
                prop_assert_eq!(Some(item.created_by), w.user(op_actor(op)).map(|u| u.id), "{}", at);
                prop_assert!(item.deleted_at.is_none(), "{}: a new item is live", at);
                prop_assert_eq!(item.external_id.clone(), ext.map(|k| format!("k{k}")), "{}", at);
                prop_assert!(
                    item.sequence_id > p.last_seq && item.sequence_id <= p.last_seq + 1 + p.failed_since,
                    "{}: sequence id {} after {} with {} failed create(s) since: must increase, never repeat, and skip \
                     at most one number per failed create",
                    at,
                    item.sequence_id,
                    p.last_seq,
                    p.failed_since
                );
                prop_assert!(item.sort_order > p.last_sort, "{}: sort_order {} after {}", at, item.sort_order, p.last_sort);
                if let Some(k) = ext {
                    w.exts.push((item.id, k));
                }
                let row = Row { id: item.id, seq: item.sequence_id, name: item.name.clone(), deleted: false };
                m.record_create(slot, row, item.sort_order, ext);
            }
            Want::Delete { slot, row } => {
                prop_assert_eq!(status, 204, "{}; body {}", at, body);
                let exts = &w.exts;
                m.record_delete(slot, row, |r| exts.iter().find(|(id, _)| *id == r.id).map(|(_, k)| *k));
            }
            Want::List { slot, with_deleted } => {
                prop_assert_eq!(status, 200, "{}; body {}", at, body);
                let items: Vec<WorkItem> =
                    serde_json::from_str(&body).map_err(|e| TestCaseError::fail(format!("{at}: {e}")))?;
                for i in &items {
                    prop_assert_eq!((i.workspace_id, i.project_id), w.scope_of(slot), "{}: a row outside its scope", at);
                }
                let got: Vec<(i64, String, bool)> =
                    items.iter().map(|i| (i.sequence_id, i.name.clone(), i.deleted_at.is_some())).collect();
                prop_assert_eq!(got, m.expected_rows(slot, with_deleted), "{}", at);
            }
        }
    }
    Ok(())
}

fn op_actor(op: &Op) -> Actor {
    match *op {
        Op::Create { actor, .. } | Op::Delete { actor, .. } | Op::List { actor, .. } => actor,
    }
}

/// Proptest config for this file: the HTTP case budget, a bounded shrink (each shrink step is a fresh run against
/// the target), and counterexamples saved beside this file (`mod.proptest-regressions`, committed per Rule 7).
fn config() -> Config {
    let cases = std::env::var("THRUST_MODEL_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(16);
    let failure_persistence: Option<Box<dyn FailurePersistence>> =
        if std::env::var_os("PROPTEST_DISABLE_FAILURE_PERSISTENCE").is_some() {
            None
        } else {
            Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions")))
        };
    Config { cases, failure_persistence, source_file: Some(file!()), max_shrink_iters: 256, ..Config::default() }
}

#[test]
fn model_work_items_over_http_match_the_reference_model() {
    let t = Target::from_env();
    assert!(
        MAX_OPS < Target::create_limit() as usize,
        "a sequence must stay under the create rate limit ({}), or the model would need to predict 429s",
        Target::create_limit()
    );
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let config = config();
    let want_cases = config.cases as usize;
    assert!(want_cases > 0, "THRUST_MODEL_CASES=0 would check nothing");
    let mut runner = TestRunner::new(config);
    let result = runner.run(&ops(), |ops| rt.block_on(run_case(&t, &ops)));
    let (cases, requests) = (CASES.load(Ordering::Relaxed), REQUESTS.load(Ordering::Relaxed));
    eprintln!("MODEL-OBSERVATION: {cases} generated sequences, {requests} requests checked against the model");
    if let Err(e) = result {
        panic!("{e}");
    }
    assert!(cases >= want_cases && requests >= cases, "the generated sequences did not run: {cases} cases, {requests} requests");
}

/// Rule 8 for the test above: a sequence generator that never produces, say, a delete of an already deleted row
/// would pass on a server that gets that case wrong. Samples sequences with a fixed seed, replays them through the
/// model alone, and requires every step class at least once.
#[test]
fn model_strategy_reaches_every_step_class() {
    let mut runner = TestRunner::deterministic();
    let mut seen = BTreeSet::new();
    for _ in 0..500 {
        let ops = ops().new_tree(&mut runner).unwrap().current();
        let mut m = Model::default();
        let mut exts = Vec::new();
        for (step, op) in ops.iter().enumerate() {
            let plan = m.plan(step, op);
            seen.insert(plan.class);
            m.simulate(step, &plan, &mut exts);
        }
    }
    let missing: Vec<_> = Class::ALL.iter().filter(|c| !seen.contains(c)).collect();
    assert!(missing.is_empty(), "never generated: {missing:?}");
}

// ---------------------------------------------------------------------------------------------
// Every actor × project × route, once
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Route {
    Create,
    List,
    ListWithDeleted,
    Delete,
    Activity,
}

const ALL_ROUTES: [Route; 5] = [Route::Create, Route::List, Route::ListWithDeleted, Route::Delete, Route::Activity];

fn min_role(r: Route) -> Role {
    match r {
        Route::Create | Route::Delete => Role::Member,
        Route::List | Route::Activity => Role::Guest,
        Route::ListWithDeleted => Role::Admin,
    }
}

fn success_status(r: Route) -> u16 {
    match r {
        Route::Create => 201,
        Route::List | Route::ListWithDeleted | Route::Activity => 200,
        Route::Delete => 204,
    }
}

/// Every combination is checked (7 × 3 × 5 = 105) and every disagreement is reported, not only the first. A delete
/// targets a fresh live item created by that project's admin, so an allowed delete and a rejected one are both
/// observable; afterwards both projects must hold exactly the rows the table predicts (no rejected request had an
/// effect).
#[tokio::test]
async fn every_actor_project_and_route_gets_the_status_of_the_access_table() {
    let t = Target::from_env();
    let w = World::seed(&t).await;
    let admin = |s: usize| if s == 0 { &w.f.admin_a } else { &w.f.admin_b };
    // per project: (name, deleted) of every row the table says must exist at the end
    let mut want_rows: [Vec<(String, bool)>; 2] = [Vec::new(), Vec::new()];
    let mut mismatches = Vec::new();
    let mut checked = 0;
    for actor in ALL_ACTORS {
        for proj in ALL_PROJECTS {
            for route in ALL_ROUTES {
                let label = format!("{actor:?} {route:?} on {proj:?}");
                let allowed = gate(actor, proj, min_role(route)).is_none();
                let mut victim = None;
                let req = match route {
                    Route::Create => Some(Req {
                        actor,
                        method: Method::POST,
                        proj,
                        item: None,
                        query: None,
                        body: Some(json!({ "name": label })),
                    }),
                    Route::List | Route::ListWithDeleted => Some(Req {
                        actor,
                        method: Method::GET,
                        proj,
                        item: None,
                        query: matches!(route, Route::ListWithDeleted).then_some("with_deleted"),
                        body: None,
                    }),
                    Route::Delete => {
                        let item = match slot(proj) {
                            Some(s) => {
                                let v = create_ok(&t, admin(s), w.project(proj), &format!("victim of {label}")).await;
                                victim = Some(s);
                                v.id
                            }
                            None => unknown_item(checked),
                        };
                        Some(Req { actor, method: Method::DELETE, proj, item: Some(item), query: None, body: None })
                    }
                    // not a work-item path, so not a `Req`
                    Route::Activity => None,
                };
                let status = match req {
                    Some(req) => w.send(&req).await.0,
                    None => {
                        let url = t.url(&format!("/api/v1/projects/{}/activity", w.project(proj)));
                        w.as_actor(t.http.get(url), actor).send().await.unwrap().status().as_u16()
                    }
                };
                let want = gate(actor, proj, min_role(route)).map_or(success_status(route), |(s, _)| s);
                if status != want {
                    mismatches.push(format!("{label}: got {status}, the table says {want}"));
                }
                if let Some(s) = victim {
                    want_rows[s].push((format!("victim of {label}"), allowed));
                }
                if matches!(route, Route::Create) && allowed {
                    want_rows[slot(proj).unwrap()].push((label.clone(), false));
                }
                checked += 1;
            }
        }
    }
    assert_eq!(checked, ALL_ACTORS.len() * ALL_PROJECTS.len() * ALL_ROUTES.len(), "the matrix did not run");
    for (s, want) in want_rows.iter_mut().enumerate() {
        let items: Vec<WorkItem> =
            list(&t, admin(s), w.scope_of(s).1, Some("with_deleted")).await.json().await.unwrap();
        let mut got: Vec<(String, bool)> = items.iter().map(|i| (i.name.clone(), i.deleted_at.is_some())).collect();
        got.sort();
        want.sort();
        if got != *want {
            mismatches.push(format!("project {s}: rows after the matrix\n  got  {got:?}\n  want {want:?}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} disagreement(s) with the access table over {checked} combinations:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
