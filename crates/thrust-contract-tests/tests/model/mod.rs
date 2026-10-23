//! Generated scenarios over HTTP (gap G-8 in `docs/src/testing/status.md`), part of the portable suite and run on
//! every target through the same `THRUST_BASE_URL` client:
//!
//! * `model_work_items_over_http_match_the_reference_model` (`main` pass, outbox fast path on): random sequences of
//!   create, delete and list by every kind of caller, in two tenants and a project that does not exist, mixed with
//!   injected and redelivered events, sweeps and activity checkpoints, checked against a reference model;
//! * `sweeper_model_outbox_delivers_what_was_swept_and_nothing_more` (`sweeper` pass, fast path off): the same
//!   sequences, where every event waits in the outbox until a sweep;
//! * `model_strategy_reaches_every_step_class`: the generator coverage control for both (Rule 8). It replays sampled
//!   sequences through the model alone and needs no target;
//! * `matrix::every_actor_project_and_route_gets_the_status_of_the_access_table`: every caller × project × route.
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
//!   exactly on deleted rows, and nothing outside the requested project (R-S1);
//! * events (ADR-0046): every successful create and delete puts exactly one event in the outbox, and a rejected or
//!   conflicting request puts none. With the fast path on the event is published at once; with it off it waits
//!   until a sweep, which publishes every waiting row (`found` = `published` = the outbox depth, nothing `failed`),
//!   and the outbox depth is always the number of waiting events. An injected event is published at once;
//! * a published event ends up as exactly one activity row in its project (verb, entity, actor), however often it
//!   is delivered: re-sending an injected event, or a real outbox event seen at an earlier checkpoint, adds nothing
//!   (the consumer is idempotent on `event_id`). A checkpoint fails at once on any row the model cannot explain (an
//!   event that should still be waiting, a rejected request's event, a duplicate) and otherwise waits up to 15 s for
//!   the missing ones. Two seconds after the run, every case's activity is read again, to catch late duplicates;
//! * attachments (ADR-0080): a put checks, in order, the name (one ordinary path segment of at most 128 bytes, no
//!   `..`; `400`), the size (at most 5 MiB, `413`), the caller's role (Member) and that the item is live in the path's
//!   project (`404`), then stores the bytes and the content type, replacing any earlier object of that name. A get
//!   (Guest) returns exactly the stored bytes and content type, or `404`; it does not look at the item, so the
//!   attachments of a soft-deleted item stay readable. A delete (Member) is `204` whether or not anything was stored.
//!   An attachment belongs to its path's project: another project's item id reaches nothing.
//!
//! Budget: `THRUST_MODEL_CASES` sequences per run (default 16 in `main`, 8 in `sweeper`), deliberately not
//! `PROPTEST_CASES`, so a `deep` run's setting cannot turn one contract cell into thousands of round trips. Each case
//! seeds its own fixture, so it starts from empty projects and fresh rate-limit buckets; `MAX_OPS` stays below the
//! create limit so the limiter is never reached (`create_is_rate_limited_per_user` owns that rule).

mod matrix;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, FailurePersistence, FileFailurePersistence, TestCaseError, TestRunner};
use reqwest::Method;
use serde_json::{json, Value};
use thrust_contract_tests::*;
use thrust_contracts::*;
use uuid::Uuid;

const MAX_OPS: usize = 28;

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

/// Which workspace slug a request names, relative to the project it addresses. The generated sequences always name
/// the project's own workspace; `matrix` covers the other two for every actor, project and route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slug {
    /// The slug of the workspace the project is in (workspace A's for a project that does not exist).
    Own,
    /// The slug of the other seeded workspace.
    Other,
    /// A slug that names no workspace.
    Unknown,
}

const ALL_SLUGS: [Slug; 3] = [Slug::Own, Slug::Other, Slug::Unknown];

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

/// The admin of a seeded project: who injects events into it and reads its activity.
fn admin_of(slot: usize) -> Actor {
    if slot == 0 {
        Actor::AdminA
    } else {
        Actor::AdminB
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

/// The verb of an injected event (outbox events are `created` and `deleted`).
#[derive(Debug, Clone, Copy)]
enum Verb {
    Updated,
    Moved,
}

impl Verb {
    fn as_str(self) -> &'static str {
        match self {
            Verb::Updated => "updated",
            Verb::Moved => "moved",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Replay {
    /// A new event id.
    Fresh,
    /// Send one of the events injected so far again (a fresh event when there are none).
    Injected(usize),
    /// Send a real outbox event again, one whose id an earlier checkpoint read from its activity row.
    Observed(usize),
}

/// The attachment size limit (`CoreConfig::max_attachment_bytes`).
const ATTACHMENT_LIMIT: usize = 5 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
enum BlobName {
    Notes,
    Data,
    /// 128 bytes, the longest valid name.
    Longest,
    /// Contains `..`.
    DoubleDot,
    /// 129 bytes.
    TooLong,
}

impl BlobName {
    fn as_string(self) -> String {
        match self {
            BlobName::Notes => "notes.txt".to_string(),
            BlobName::Data => "data.bin".to_string(),
            BlobName::Longest => "x".repeat(128),
            BlobName::DoubleDot => "a..b".to_string(),
            BlobName::TooLong => "x".repeat(129),
        }
    }

    fn valid(self) -> bool {
        match self {
            BlobName::Notes | BlobName::Data | BlobName::Longest => true,
            BlobName::DoubleDot | BlobName::TooLong => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Size {
    Empty,
    /// 1..=64 bytes.
    Small(u8),
    /// Exactly the limit.
    AtLimit,
    /// One byte over it.
    OverLimit,
}

impl Size {
    fn len(self) -> usize {
        match self {
            Size::Empty => 0,
            Size::Small(n) => usize::from(n),
            Size::AtLimit => ATTACHMENT_LIMIT,
            Size::OverLimit => ATTACHMENT_LIMIT + 1,
        }
    }

    /// Binary content (zero bytes included) that differs by `seed`, so a replaced object is distinguishable.
    fn bytes(self, seed: u8) -> Vec<u8> {
        (0..self.len()).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
    }
}

#[derive(Debug, Clone, Copy)]
enum Ctype {
    Text,
    Custom,
}

impl Ctype {
    fn as_str(self) -> &'static str {
        match self {
            Ctype::Text => "text/plain",
            Ctype::Custom => "application/x-thrust",
        }
    }
}

#[derive(Debug, Clone)]
enum Op {
    /// `ext`: an external key from a small domain, so that keys collide.
    Create { actor: Actor, proj: Proj, name: Name, ext: Option<u8> },
    Delete { actor: Actor, proj: Proj, pick: Pick },
    List { actor: Actor, proj: Proj, scope: Scope },
    /// Put an envelope straight on the queue (`/internal/inject-event`), about one of the project's items.
    Inject { proj: Proj, item: usize, verb: Verb, replay: Replay },
    /// Run the outbox sweeper (`/internal/sweep`), what cron does.
    Sweep,
    /// Read both projects' activity and compare it with the model.
    Checkpoint,
    PutAttachment { actor: Actor, proj: Proj, pick: Pick, name: BlobName, size: Size, seed: u8, ctype: Option<Ctype> },
    GetAttachment { actor: Actor, proj: Proj, pick: Pick, name: BlobName },
    DeleteAttachment { actor: Actor, proj: Proj, pick: Pick, name: BlobName },
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

/// A project and a caller: usually one who may write there, so sequences build up state worth checking, otherwise
/// anyone (drawn independently, the pair is mostly a cross-tenant or unauthenticated rejection).
fn caller() -> impl Strategy<Value = (Actor, Proj)> {
    proj().prop_flat_map(|p| {
        let writers = match p {
            Proj::A => vec![Actor::AdminA, Actor::MemberA],
            Proj::B | Proj::Unknown => vec![Actor::AdminB],
        };
        (prop_oneof![3 => prop::sample::select(writers), 2 => actor()], Just(p))
    })
}

fn seeded() -> impl Strategy<Value = Proj> {
    prop_oneof![2 => Just(Proj::A), 1 => Just(Proj::B)]
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

fn verb() -> impl Strategy<Value = Verb> {
    prop_oneof![Just(Verb::Updated), Just(Verb::Moved)]
}

fn replay() -> impl Strategy<Value = Replay> {
    prop_oneof![2 => Just(Replay::Fresh), 1 => (0..8usize).prop_map(Replay::Injected), 1 => (0..8usize).prop_map(Replay::Observed)]
}

fn blob_name() -> impl Strategy<Value = BlobName> {
    prop_oneof![
        4 => Just(BlobName::Notes),
        3 => Just(BlobName::Data),
        1 => Just(BlobName::Longest),
        1 => Just(BlobName::DoubleDot),
        1 => Just(BlobName::TooLong),
    ]
}

fn size() -> impl Strategy<Value = Size> {
    // the two 5 MiB sizes are rare: each one is a real 5 MiB upload
    prop_oneof![8 => (1..=64u8).prop_map(Size::Small), 1 => Just(Size::Empty), 1 => Just(Size::AtLimit), 1 => Just(Size::OverLimit)]
}

fn ctype() -> impl Strategy<Value = Option<Ctype>> {
    prop::option::weighted(0.6, prop_oneof![Just(Ctype::Text), Just(Ctype::Custom)])
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        5 => (caller(), name(), prop::option::weighted(0.4, 0..3u8))
            .prop_map(|((actor, proj), name, ext)| Op::Create { actor, proj, name, ext }),
        3 => (caller(), pick()).prop_map(|((actor, proj), pick)| Op::Delete { actor, proj, pick }),
        1 => (caller(), scope()).prop_map(|((actor, proj), scope)| Op::List { actor, proj, scope }),
        2 => (seeded(), 0..16usize, verb(), replay())
            .prop_map(|(proj, item, verb, replay)| Op::Inject { proj, item, verb, replay }),
        1 => Just(Op::Sweep),
        1 => Just(Op::Checkpoint),
        3 => (caller(), pick(), blob_name(), size(), any::<u8>(), ctype()).prop_map(
            |((actor, proj), pick, name, size, seed, ctype)| Op::PutAttachment { actor, proj, pick, name, size, seed, ctype }
        ),
        3 => (caller(), pick(), blob_name())
            .prop_map(|((actor, proj), pick, name)| Op::GetAttachment { actor, proj, pick, name }),
        2 => (caller(), pick(), blob_name())
            .prop_map(|((actor, proj), pick, name)| Op::DeleteAttachment { actor, proj, pick, name }),
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
    Injected,
    RedeliveredInjected,
    RedeliveredOutboxEvent,
    /// A sweep with nothing waiting (always, with the fast path on).
    Swept,
    /// A sweep that publishes waiting events (fast path off only).
    SweptPending,
    Checked,
    AttachmentStored,
    /// A put over an existing object of the same name.
    AttachmentReplaced,
    /// A put of exactly the size limit.
    AttachmentAtLimit,
    AttachmentTooLarge,
    AttachmentBadName,
    AttachmentOnDeletedItem,
    AttachmentOnOtherProjectsItem,
    AttachmentOnUnknownItem,
    AttachmentRead,
    /// A get of an attachment whose item was soft-deleted afterwards.
    AttachmentReadAfterItemDeleted,
    AttachmentMissing,
    /// A get through one project of an object stored under another project's item.
    AttachmentCrossProject,
    AttachmentDeleted,
    AttachmentDeleteMissing,
}

impl Class {
    const ALL: [Class; 37] = [
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
        Class::Injected,
        Class::RedeliveredInjected,
        Class::RedeliveredOutboxEvent,
        Class::Swept,
        Class::SweptPending,
        Class::Checked,
        Class::AttachmentStored,
        Class::AttachmentReplaced,
        Class::AttachmentAtLimit,
        Class::AttachmentTooLarge,
        Class::AttachmentBadName,
        Class::AttachmentOnDeletedItem,
        Class::AttachmentOnOtherProjectsItem,
        Class::AttachmentOnUnknownItem,
        Class::AttachmentRead,
        Class::AttachmentReadAfterItemDeleted,
        Class::AttachmentMissing,
        Class::AttachmentCrossProject,
        Class::AttachmentDeleted,
        Class::AttachmentDeleteMissing,
    ];
}

#[derive(Debug, Clone)]
struct Row {
    id: WorkItemId,
    seq: i64,
    name: String,
    ext: Option<u8>,
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
    /// Stored attachments by (item, name).
    blobs: BTreeMap<(WorkItemId, String), Stored>,
}

/// An attachment as the model keeps it: how to make its bytes, not the bytes (two of the sizes are 5 MiB).
#[derive(Debug, Clone, PartialEq)]
struct Stored {
    size: Size,
    seed: u8,
    content_type: Option<&'static str>,
}

impl Stored {
    fn bytes(&self) -> Vec<u8> {
        self.size.bytes(self.seed)
    }
}

/// Whose item a request names.
#[derive(Debug, Clone, Copy)]
enum ItemRef {
    /// A row of the path's project, by index.
    Own(usize),
    /// An item of the other seeded project, by slot.
    Other(usize),
    Unknown,
}

/// One event the model expects to become one activity row.
#[derive(Debug, Clone)]
struct Event {
    slot: usize,
    /// Known for injected events, and for outbox events once a checkpoint has matched their row.
    id: Option<EventId>,
    entity: Uuid,
    verb: &'static str,
    actor: Actor,
    injected: bool,
    /// On the queue (or delivered); `false` while it waits in the outbox for a sweep.
    published: bool,
}

#[derive(Debug)]
struct Model {
    projects: [ProjectModel; 2],
    /// Whether the target publishes events right after commit (`main` pass) or leaves them to the sweeper.
    fastpath: bool,
    events: Vec<Event>,
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
    Create { slot: usize, name: String, ext: Option<u8>, actor: Actor },
    Delete { slot: usize, row: usize, actor: Actor },
    List { slot: usize, with_deleted: bool },
    /// `202`; the new event, or `None` for a redelivery (which must add nothing).
    Inject(Option<Event>),
    Sweep,
    Checkpoint,
    PutAttachment { slot: usize, key: (WorkItemId, String), stored: Stored },
    GetAttachment(Stored),
    DeleteAttachment { slot: usize, key: (WorkItemId, String) },
}

/// One work-item request, in model terms; `World::send` turns it into ids.
#[derive(Debug, Clone)]
struct Req {
    actor: Actor,
    method: Method,
    proj: Proj,
    slug: Slug,
    item: Option<WorkItemId>,
    query: Option<&'static str>,
    body: Option<Value>,
    /// `.../work-items/<item>/attachments/<name>`.
    attachment: Option<String>,
    /// The body and content type of an attachment put.
    upload: Option<Stored>,
}

impl Req {
    fn bare(actor: Actor, method: Method, proj: Proj) -> Req {
        Req { actor, method, proj, slug: Slug::Own, item: None, query: None, body: None, attachment: None, upload: None }
    }
}

/// A response, as far as the model looks at it.
struct Resp {
    status: u16,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Resp {
    /// The body for a failure message, cut short (an attachment can be 5 MiB).
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body[..self.body.len().min(300)]).into_owned()
    }
}

#[derive(Debug, Clone)]
enum Call {
    Api(Req),
    Inject(Event),
    Sweep,
    Checkpoint,
}

struct Plan {
    call: Call,
    want: Want,
    class: Class,
}

fn unknown_item(step: usize) -> WorkItemId {
    WorkItemId(Uuid::from_u128(0xBAD0_0000 + step as u128))
}

/// What a checkpoint read, against the model.
enum Seen {
    /// Every row explained and every published event present; the ids learnt for outbox events.
    Complete(Vec<(usize, EventId)>),
    /// Every row explained, some published events not delivered yet.
    Missing(Vec<String>),
}

impl Model {
    fn new(fastpath: bool) -> Model {
        Model { projects: Default::default(), fastpath, events: Vec::new() }
    }

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
                let call = Call::Api(Req { body: Some(body), ..Req::bare(actor, Method::POST, proj) });
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
                            (Want::Create { slot: s, name: raw.trim().to_string(), ext, actor }, class)
                        }
                    }
                };
                Plan { call, want, class }
            }
            Op::Delete { actor, proj, pick } => {
                let (item, target) = self.resolve(step, proj, pick);
                let call = Call::Api(Req { item: Some(item), ..Req::bare(actor, Method::DELETE, proj) });
                let (want, class) = if let Some((status, class)) = gate(actor, proj, Role::Member) {
                    (Want::Reject(status), class)
                } else {
                    let s = slot(proj).expect("an authorized project is a seeded one");
                    match target {
                        ItemRef::Own(r) if !self.projects[s].rows[r].deleted => {
                            (Want::Delete { slot: s, row: r, actor }, Class::Deleted)
                        }
                        ItemRef::Own(_) => (Want::Reject(404), Class::DeleteAlreadyDeleted),
                        ItemRef::Other(_) => (Want::Reject(404), Class::DeleteOtherProjectsItem),
                        ItemRef::Unknown => (Want::Reject(404), Class::DeleteUnknownId),
                    }
                };
                Plan { call, want, class }
            }
            Op::List { actor, proj, scope } => {
                let query = match scope {
                    Scope::Default => None,
                    Scope::Visible => Some("visible"),
                    Scope::WithDeleted => Some("with_deleted"),
                    Scope::Bogus => Some("bogus"),
                };
                let call = Call::Api(Req { query, ..Req::bare(actor, Method::GET, proj) });
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
                Plan { call, want, class }
            }
            Op::Inject { proj, item, verb, replay } => {
                let s = slot(proj).unwrap_or(0);
                // an event already sent, injected or not, whose id is known
                let earlier = |injected: bool, i: usize| {
                    let known: Vec<usize> = (0..self.events.len())
                        .filter(|&n| self.events[n].injected == injected && self.events[n].id.is_some())
                        .collect();
                    (!known.is_empty()).then(|| known[i % known.len()])
                };
                let again = match replay {
                    Replay::Fresh => None,
                    Replay::Injected(i) => earlier(true, i).map(|n| (n, Class::RedeliveredInjected)),
                    Replay::Observed(i) => earlier(false, i).map(|n| (n, Class::RedeliveredOutboxEvent)),
                };
                match again {
                    Some((n, class)) => Plan { call: Call::Inject(self.events[n].clone()), want: Want::Inject(None), class },
                    None => {
                        let rows = &self.projects[s].rows;
                        let entity =
                            if rows.is_empty() { unknown_item(step) } else { rows[item % rows.len()].id }.as_uuid();
                        let ev = Event {
                            slot: s,
                            id: Some(EventId(Uuid::new_v4())),
                            entity,
                            verb: verb.as_str(),
                            actor: admin_of(s),
                            injected: true,
                            published: true,
                        };
                        Plan { call: Call::Inject(ev.clone()), want: Want::Inject(Some(ev)), class: Class::Injected }
                    }
                }
            }
            Op::Sweep => {
                let class = if self.pending() > 0 { Class::SweptPending } else { Class::Swept };
                Plan { call: Call::Sweep, want: Want::Sweep, class }
            }
            Op::Checkpoint => Plan { call: Call::Checkpoint, want: Want::Checkpoint, class: Class::Checked },
            Op::PutAttachment { actor, proj, pick, name, size, seed, ctype } => {
                let (item, target) = self.resolve(step, proj, pick);
                let stored = Stored { size, seed, content_type: ctype.map(Ctype::as_str) };
                let call = Call::Api(Req {
                    item: Some(item),
                    attachment: Some(name.as_string()),
                    upload: Some(stored.clone()),
                    ..Req::bare(actor, Method::PUT, proj)
                });
                let (want, class) = if matches!(standing(actor, proj), Standing::Unauthenticated) {
                    (Want::Reject(401), Class::Unauthenticated)
                } else if !name.valid() {
                    (Want::Reject(400), Class::AttachmentBadName)
                } else if matches!(size, Size::OverLimit) {
                    (Want::Reject(413), Class::AttachmentTooLarge)
                } else if let Some((status, class)) = gate(actor, proj, Role::Member) {
                    (Want::Reject(status), class)
                } else {
                    let s = slot(proj).expect("an authorized project is a seeded one");
                    match target {
                        ItemRef::Own(r) if !self.projects[s].rows[r].deleted => {
                            let key = (item, name.as_string());
                            let class = if self.projects[s].blobs.contains_key(&key) {
                                Class::AttachmentReplaced
                            } else if matches!(size, Size::AtLimit) {
                                Class::AttachmentAtLimit
                            } else {
                                Class::AttachmentStored
                            };
                            (Want::PutAttachment { slot: s, key, stored }, class)
                        }
                        ItemRef::Own(_) => (Want::Reject(404), Class::AttachmentOnDeletedItem),
                        ItemRef::Other(_) => (Want::Reject(404), Class::AttachmentOnOtherProjectsItem),
                        ItemRef::Unknown => (Want::Reject(404), Class::AttachmentOnUnknownItem),
                    }
                };
                Plan { call, want, class }
            }
            Op::GetAttachment { actor, proj, pick, name } => {
                let (item, target) = self.resolve(step, proj, pick);
                let call = Call::Api(Req {
                    item: Some(item),
                    attachment: Some(name.as_string()),
                    ..Req::bare(actor, Method::GET, proj)
                });
                let key = (item, name.as_string());
                let (want, class) = if matches!(standing(actor, proj), Standing::Unauthenticated) {
                    (Want::Reject(401), Class::Unauthenticated)
                } else if !name.valid() {
                    (Want::Reject(400), Class::AttachmentBadName)
                } else if let Some((status, class)) = gate(actor, proj, Role::Guest) {
                    (Want::Reject(status), class)
                } else {
                    let s = slot(proj).expect("an authorized project is a seeded one");
                    match (self.projects[s].blobs.get(&key), target) {
                        (Some(b), ItemRef::Own(r)) if self.projects[s].rows[r].deleted => {
                            (Want::GetAttachment(b.clone()), Class::AttachmentReadAfterItemDeleted)
                        }
                        (Some(b), _) => (Want::GetAttachment(b.clone()), Class::AttachmentRead),
                        (None, ItemRef::Other(o)) if self.projects[o].blobs.contains_key(&key) => {
                            (Want::Reject(404), Class::AttachmentCrossProject)
                        }
                        (None, _) => (Want::Reject(404), Class::AttachmentMissing),
                    }
                };
                Plan { call, want, class }
            }
            Op::DeleteAttachment { actor, proj, pick, name } => {
                let (item, _) = self.resolve(step, proj, pick);
                let call = Call::Api(Req {
                    item: Some(item),
                    attachment: Some(name.as_string()),
                    ..Req::bare(actor, Method::DELETE, proj)
                });
                let (want, class) = if matches!(standing(actor, proj), Standing::Unauthenticated) {
                    (Want::Reject(401), Class::Unauthenticated)
                } else if !name.valid() {
                    (Want::Reject(400), Class::AttachmentBadName)
                } else if let Some((status, class)) = gate(actor, proj, Role::Member) {
                    (Want::Reject(status), class)
                } else {
                    let s = slot(proj).expect("an authorized project is a seeded one");
                    let key = (item, name.as_string());
                    let class = if self.projects[s].blobs.contains_key(&key) {
                        Class::AttachmentDeleted
                    } else {
                        Class::AttachmentDeleteMissing
                    };
                    (Want::DeleteAttachment { slot: s, key }, class)
                };
                Plan { call, want, class }
            }
        }
    }

    /// The item `pick` names in a request on `proj`: its id, and whose row it is.
    fn resolve(&self, step: usize, proj: Proj, pick: Pick) -> (WorkItemId, ItemRef) {
        let other = match proj {
            Proj::A | Proj::Unknown => 1,
            Proj::B => 0,
        };
        let rows = |s: usize| &self.projects[s].rows;
        match pick {
            Pick::Own(i) => match slot(proj) {
                Some(s) if !rows(s).is_empty() => {
                    let r = i % rows(s).len();
                    (rows(s)[r].id, ItemRef::Own(r))
                }
                _ => (unknown_item(step), ItemRef::Unknown),
            },
            Pick::OtherProject(i) if !rows(other).is_empty() => {
                (rows(other)[i % rows(other).len()].id, ItemRef::Other(other))
            }
            Pick::OtherProject(_) | Pick::Unknown => (unknown_item(step), ItemRef::Unknown),
        }
    }

    fn record_conflict(&mut self, slot: usize) {
        self.projects[slot].failed_since += 1;
    }

    fn record_create(&mut self, slot: usize, row: Row, sort: i64, actor: Actor) {
        let entity = row.id.as_uuid();
        let p = &mut self.projects[slot];
        p.last_seq = row.seq;
        p.last_sort = sort;
        p.failed_since = 0;
        if let Some(k) = row.ext {
            p.live_ext.insert(k);
            p.deleted_ext.remove(&k);
        }
        p.rows.push(row);
        self.emit(slot, entity, "created", actor);
    }

    fn record_delete(&mut self, slot: usize, row: usize, actor: Actor) {
        let p = &mut self.projects[slot];
        p.rows[row].deleted = true;
        if let Some(k) = p.rows[row].ext {
            p.live_ext.remove(&k);
            p.deleted_ext.insert(k);
        }
        let entity = p.rows[row].id.as_uuid();
        self.emit(slot, entity, "deleted", actor);
    }

    /// The outbox row a committed create or delete writes.
    fn emit(&mut self, slot: usize, entity: Uuid, verb: &'static str, actor: Actor) {
        let published = self.fastpath;
        self.events.push(Event { slot, id: None, entity, verb, actor, injected: false, published });
    }

    /// Every event whose id is known, sent again, then a checkpoint: the consumer must add nothing.
    fn redeliver_everything(&self) -> Vec<Op> {
        let known = |injected: bool| self.events.iter().filter(|e| e.injected == injected && e.id.is_some()).count();
        let again = |replay| Op::Inject { proj: Proj::A, item: 0, verb: Verb::Updated, replay };
        (0..known(true))
            .map(|i| again(Replay::Injected(i)))
            .chain((0..known(false)).map(|i| again(Replay::Observed(i))))
            .chain([Op::Checkpoint])
            .collect()
    }

    /// Events waiting in the outbox.
    fn pending(&self) -> usize {
        self.events.iter().filter(|e| !e.published).count()
    }

    fn publish_all(&mut self) {
        for e in &mut self.events {
            e.published = true;
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

    /// Explains each activity row of `slot` by a distinct published event: by id where the model knows it, otherwise
    /// by entity, verb and actor. `Err` names the first row nothing explains, or an event id with two rows.
    fn match_activity(
        &self,
        slot: usize,
        rows: &[Activity],
        user_id: impl Fn(Actor) -> Option<UserId>,
    ) -> Result<Seen, String> {
        let expected: Vec<usize> =
            (0..self.events.len()).filter(|&n| self.events[n].slot == slot && self.events[n].published).collect();
        let mut used = vec![false; self.events.len()];
        let mut ids = BTreeSet::new();
        let mut learnt = Vec::new();
        for r in rows {
            if !ids.insert(r.event_id) {
                return Err(format!("event {} has two activity rows (a redelivery was not deduplicated)", r.event_id));
            }
            let same = |e: &Event| e.entity == r.entity_id && e.verb == r.verb && user_id(e.actor) == r.actor_id;
            let hit = expected.iter().copied().find(|&n| {
                !used[n]
                    && match self.events[n].id {
                        Some(id) => id == r.event_id,
                        None => same(&self.events[n]),
                    }
            });
            match hit {
                Some(n) if same(&self.events[n]) => {
                    used[n] = true;
                    if self.events[n].id.is_none() {
                        learnt.push((n, r.event_id));
                    }
                }
                Some(n) => {
                    return Err(format!("event {} was recorded as {r:?}, the model sent {:?}", r.event_id, self.events[n]))
                }
                None => {
                    return Err(format!(
                        "an activity row the model did not predict (an event still waiting in the outbox, one from a \
                         rejected request, or a duplicate): {r:?}"
                    ))
                }
            }
        }
        let missing: Vec<String> =
            expected.iter().filter(|&&n| !used[n]).map(|&n| format!("{:?}", self.events[n])).collect();
        Ok(if missing.is_empty() { Seen::Complete(learnt) } else { Seen::Missing(missing) })
    }

    /// Offline replay for the coverage control: apply the predicted effect with made-up ids.
    fn simulate(&mut self, step: usize, plan: &Plan) {
        match plan.want {
            Want::Reject(_) | Want::List { .. } => {}
            Want::Conflict { slot, .. } => self.record_conflict(slot),
            Want::Create { slot, ref name, ext, actor } => {
                let id = WorkItemId(Uuid::from_u128(step as u128 + 1));
                let seq = self.projects[slot].last_seq + 1;
                self.record_create(slot, Row { id, seq, name: name.clone(), ext, deleted: false }, seq * 10_000, actor);
            }
            Want::Delete { slot, row, actor } => self.record_delete(slot, row, actor),
            Want::Inject(ref fresh) => self.events.extend(fresh.clone()),
            Want::Sweep => self.publish_all(),
            Want::PutAttachment { slot, ref key, ref stored } => {
                self.projects[slot].blobs.insert(key.clone(), stored.clone());
            }
            Want::GetAttachment(_) => {}
            Want::DeleteAttachment { slot, ref key } => {
                self.projects[slot].blobs.remove(key);
            }
            Want::Checkpoint => {
                for e in &mut self.events {
                    if e.published && e.id.is_none() {
                        e.id = Some(EventId(Uuid::new_v4()));
                    }
                }
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
}

impl<'a> World<'a> {
    async fn seed(t: &'a Target) -> World<'a> {
        let f = Fixture::seed(t).await;
        World { t, f, unknown: ProjectId(Uuid::new_v4()) }
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
            Proj::A => self.f.a.project,
            Proj::B => self.f.b.project,
            Proj::Unknown => self.unknown,
        }
    }

    /// The seeded project in `slot`, through its own workspace's slug.
    fn at_slot(&self, slot: usize) -> &At {
        if slot == 0 {
            &self.f.a
        } else {
            &self.f.b
        }
    }

    /// `p` addressed through the slug `slug` names for it.
    fn at(&self, p: Proj, slug: Slug) -> At {
        let (own, other) = match p {
            Proj::A | Proj::Unknown => (&self.f.a, &self.f.b),
            Proj::B => (&self.f.b, &self.f.a),
        };
        let slug = match slug {
            Slug::Own => own.slug.clone(),
            Slug::Other => other.slug.clone(),
            Slug::Unknown => "no-such-workspace".to_string(),
        };
        At { slug, project: self.project(p) }
    }

    fn scope_of(&self, slot: usize) -> (WorkspaceId, ProjectId) {
        if slot == 0 {
            (self.f.ws_a, self.f.a.project)
        } else {
            (self.f.ws_b, self.f.b.project)
        }
    }

    fn as_actor(&self, b: reqwest::RequestBuilder, actor: Actor) -> reqwest::RequestBuilder {
        match actor {
            Actor::Anonymous => b,
            Actor::BadToken => b.bearer_auth("not-a-session"),
            a => b.bearer_auth(&self.user(a).expect("an authenticated actor has a user").token),
        }
    }

    async fn send(&self, req: &Req) -> Resp {
        self.try_send(req).await.unwrap()
    }

    async fn try_send(&self, req: &Req) -> Result<Resp, reqwest::Error> {
        let mut path = wi_path(&self.at(req.proj, req.slug));
        if let Some(id) = req.item {
            path.push_str(&format!("/{id}"));
        }
        if let Some(name) = &req.attachment {
            path.push_str(&format!("/attachments/{name}"));
        }
        if let Some(q) = req.query {
            path.push_str(&format!("?scope={q}"));
        }
        let mut b = self.as_actor(self.t.http.request(req.method.clone(), self.t.url(&path)), req.actor);
        if let Some(body) = &req.body {
            b = b.json(body);
        }
        if let Some(up) = &req.upload {
            b = b.body(up.bytes());
            if let Some(ct) = up.content_type {
                b = b.header("content-type", ct);
            }
        }
        let r = b.send().await?;
        let status = r.status().as_u16();
        let content_type = r.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_string);
        Ok(Resp { status, content_type, body: r.bytes().await.map(|b| b.to_vec()).unwrap_or_default() })
    }

    /// The envelope a producer would put on the queue for `ev`.
    fn envelope(&self, ev: &Event) -> Value {
        let (ws, project) = self.scope_of(ev.slot);
        json!({
            "event_id": ev.id.expect("an event sent to the queue has an id"),
            "schema_version": 1,
            "workspace_id": ws,
            "event_type": format!("work_item.{}", ev.verb),
            "entity_type": "work_item",
            "entity_id": ev.entity,
            "actor_id": self.user(ev.actor).map(|u| u.id),
            "occurred_at": "2026-10-18T00:00:00Z",
            "payload": { "project_id": project, "changed_fields": ["name"] },
            "trace_id": null
        })
    }

    /// A project's activity, read by its admin.
    async fn activity(&self, slot: usize) -> Result<Vec<Activity>, String> {
        let url = self.t.url(&format!("{}/activity", project_path(self.at_slot(slot))));
        let r = self.as_actor(self.t.http.get(url), admin_of(slot)).send().await.unwrap();
        let status = r.status().as_u16();
        let body = r.text().await.unwrap_or_default();
        if status != 200 {
            return Err(format!("activity of project {slot}: {status} {body}"));
        }
        serde_json::from_str(&body).map_err(|e| format!("activity of project {slot}: {e}"))
    }
}

/// Sequences run and requests sent, shrinking included: the test fails if it compared nothing. The counts are printed
/// as `MODEL-OBSERVATION` (visible on failure, or with `--nocapture`).
#[derive(Default)]
struct Counts {
    cases: AtomicUsize,
    requests: AtomicUsize,
}

/// One case's activity after its last checkpoint, read again once the whole run is over.
struct Settle {
    at: At,
    token: String,
    event_ids: Vec<EventId>,
}

fn fail(at: &str, e: impl std::fmt::Display) -> TestCaseError {
    TestCaseError::fail(format!("{at}: {e}"))
}

async fn run_case(
    t: &Target,
    ops: &[Op],
    fastpath: bool,
    counts: &Counts,
    settle: &Mutex<Vec<Settle>>,
) -> Result<(), TestCaseError> {
    counts.cases.fetch_add(1, Ordering::Relaxed);
    let w = World::seed(t).await;
    let mut m = Model::new(fastpath);
    // Undispatched rows other tests left behind; a sweep takes them too. Only read with the fast path off, where the
    // pass runs one test at a time.
    let mut leftover = if fastpath { 0 } else { t.outbox_depth().await };
    // Every case ends with a checkpoint; with the fast path off, a sweep first so that everything is delivered. Then
    // every event whose id is now known is sent again and checked once more, so that each case exercises redelivery
    // whatever the generator drew (the read after the run catches a duplicate that lands late).
    let mut all: Vec<Op> = ops.to_vec();
    all.extend(if fastpath { vec![Op::Checkpoint] } else { vec![Op::Sweep, Op::Checkpoint] });
    let mut redelivered = false;
    let mut next = 0;
    loop {
        if next == all.len() {
            if redelivered {
                break;
            }
            redelivered = true;
            all.extend(m.redeliver_everything());
        }
        let (step, op) = (next, all[next].clone());
        next += 1;
        let op = &op;
        let plan = m.plan(step, op);
        counts.requests.fetch_add(1, Ordering::Relaxed);
        let at = format!("step {step}: {op:?} ({:?})", plan.class);
        match (plan.call, plan.want) {
            (Call::Api(req), want) => {
                let resp = match w.try_send(&req).await {
                    Ok(resp) => resp,
                    // An upload without a valid session is refused before its body is read: the server answers 401
                    // and closes the connection while a large body is still being sent, so the client may see the
                    // connection close instead of the 401. Only that case, and only for a large body.
                    Err(_)
                        if matches!(want, Want::Reject(401))
                            && req.upload.as_ref().is_some_and(|u| u.size.len() > 64 * 1024) =>
                    {
                        continue;
                    }
                    Err(e) => return Err(fail(&at, e)),
                };
                let (status, body) = (resp.status, resp.text());
                match want {
                    Want::Reject(want) => prop_assert_eq!(status, want, "{}; body {}", at, body),
                    Want::Conflict { slot, ext } => {
                        prop_assert_eq!(status, 409, "{}: external key k{} is live; body {}", at, ext, body);
                        m.record_conflict(slot);
                    }
                    Want::Create { slot, name, ext, actor } => {
                        prop_assert_eq!(status, 201, "{}; body {}", at, body);
                        let item: WorkItem = serde_json::from_slice(&resp.body).map_err(|e| fail(&at, e))?;
                        let p = &m.projects[slot];
                        prop_assert_eq!((item.workspace_id, item.project_id), w.scope_of(slot), "{}: created outside its scope", at);
                        prop_assert_eq!(&item.name, &name, "{}: the stored name is the trimmed name", at);
                        prop_assert_eq!(Some(item.created_by), w.user(actor).map(|u| u.id), "{}", at);
                        prop_assert!(item.deleted_at.is_none(), "{}: a new item is live", at);
                        prop_assert_eq!(item.external_id.clone(), ext.map(|k| format!("k{k}")), "{}", at);
                        prop_assert!(
                            item.sequence_id > p.last_seq && item.sequence_id <= p.last_seq + 1 + p.failed_since,
                            "{}: sequence id {} after {} with {} failed create(s) since: must increase, never repeat, \
                             and skip at most one number per failed create",
                            at,
                            item.sequence_id,
                            p.last_seq,
                            p.failed_since
                        );
                        prop_assert!(item.sort_order > p.last_sort, "{}: sort_order {} after {}", at, item.sort_order, p.last_sort);
                        let row = Row { id: item.id, seq: item.sequence_id, name: item.name, ext, deleted: false };
                        m.record_create(slot, row, item.sort_order, actor);
                    }
                    Want::Delete { slot, row, actor } => {
                        prop_assert_eq!(status, 204, "{}; body {}", at, body);
                        m.record_delete(slot, row, actor);
                    }
                    Want::List { slot, with_deleted } => {
                        prop_assert_eq!(status, 200, "{}; body {}", at, body);
                        let items: Vec<WorkItem> = serde_json::from_slice(&resp.body).map_err(|e| fail(&at, e))?;
                        for i in &items {
                            prop_assert_eq!((i.workspace_id, i.project_id), w.scope_of(slot), "{}: a row outside its scope", at);
                        }
                        let got: Vec<(i64, String, bool)> =
                            items.iter().map(|i| (i.sequence_id, i.name.clone(), i.deleted_at.is_some())).collect();
                        prop_assert_eq!(got, m.expected_rows(slot, with_deleted), "{}", at);
                    }
                    Want::PutAttachment { slot, key, stored } => {
                        prop_assert_eq!(status, 204, "{}; body {}", at, body);
                        m.projects[slot].blobs.insert(key, stored);
                    }
                    Want::GetAttachment(stored) => {
                        prop_assert_eq!(status, 200, "{}; body {}", at, body);
                        let want = stored.bytes();
                        prop_assert_eq!(resp.body.len(), want.len(), "{}: attachment length", at);
                        prop_assert!(resp.body == want, "{}: the attachment's bytes differ from what was stored", at);
                        prop_assert_eq!(resp.content_type.as_deref(), stored.content_type, "{}: content type", at);
                    }
                    Want::DeleteAttachment { slot, key } => {
                        prop_assert_eq!(status, 204, "{}; body {}", at, body);
                        m.projects[slot].blobs.remove(&key);
                    }
                    other => unreachable!("a work-item request planned {other:?}"),
                }
            }
            (Call::Inject(ev), Want::Inject(fresh)) => {
                let r = t.admin_post("/internal/inject-event", &w.envelope(&ev)).await;
                let status = r.status().as_u16();
                prop_assert_eq!(status, 202, "{}; body {}", at, r.text().await.unwrap_or_default());
                m.events.extend(fresh);
            }
            (Call::Sweep, Want::Sweep) => {
                let r = t.admin_post("/internal/sweep", &json!({})).await;
                let status = r.status().as_u16();
                let body = r.text().await.unwrap_or_default();
                prop_assert_eq!(status, 200, "{}; body {}", at, body);
                if !fastpath {
                    let rep: Value = serde_json::from_str(&body).map_err(|e| fail(&at, e))?;
                    let waiting = leftover + m.pending() as u64;
                    let got = (rep["found"].as_u64(), rep["published"].as_u64(), rep["failed"].as_u64());
                    prop_assert_eq!(got, (Some(waiting), Some(waiting), Some(0)), "{}: the sweep must publish every waiting row", at);
                    leftover = 0;
                }
                m.publish_all();
            }
            (Call::Checkpoint, Want::Checkpoint) => {
                for s in 0..2 {
                    let deadline = Instant::now() + Duration::from_secs(15);
                    loop {
                        let rows = w.activity(s).await.map_err(|e| fail(&at, e))?;
                        for r in &rows {
                            prop_assert_eq!((r.workspace_id, r.project_id), w.scope_of(s), "{}: activity outside its scope", at);
                        }
                        match m.match_activity(s, &rows, |a| w.user(a).map(|u| u.id)) {
                            Err(e) => return Err(fail(&at, format!("project {s}: {e}"))),
                            Ok(Seen::Complete(learnt)) => {
                                for (n, id) in learnt {
                                    m.events[n].id = Some(id);
                                }
                                break;
                            }
                            Ok(Seen::Missing(missing)) if Instant::now() > deadline => {
                                return Err(fail(&at, format!("project {s}: never delivered within 15 s: {missing:?}")))
                            }
                            Ok(Seen::Missing(_)) => tokio::time::sleep(Duration::from_millis(100)).await,
                        }
                    }
                }
                if !fastpath {
                    let depth = t.outbox_depth().await;
                    prop_assert_eq!(depth, leftover + m.pending() as u64, "{}: outbox depth is the number of waiting events", at);
                }
            }
            (call, want) => unreachable!("planned {call:?} with {want:?}"),
        }
    }
    let mut late = settle.lock().unwrap();
    for s in 0..2 {
        let mut event_ids: Vec<EventId> =
            m.events.iter().filter(|e| e.slot == s).map(|e| e.id.expect("the last checkpoint matched every event")).collect();
        event_ids.sort();
        let token = w.user(admin_of(s)).unwrap().token.clone();
        late.push(Settle { at: w.at_slot(s).clone(), token, event_ids });
    }
    Ok(())
}

/// Proptest config for this file: the HTTP case budget, a bounded shrink (each shrink step is a fresh run against
/// the target, and one that waits for an undelivered event takes 15 s, so the shrink also stops after a minute), and
/// counterexamples saved beside this file (`mod.proptest-regressions`, committed per Rule 7).
fn config(default_cases: u32) -> Config {
    let cases = std::env::var("THRUST_MODEL_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(default_cases);
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
        max_shrink_iters: 256,
        max_shrink_time: 60_000,
        ..Config::default()
    }
}

fn run_model(fastpath: bool, default_cases: u32) {
    let t = Target::from_env();
    assert!(
        MAX_OPS < Target::create_limit() as usize,
        "a sequence must stay under the create rate limit ({}), or the model would need to predict 429s",
        Target::create_limit()
    );
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let config = config(default_cases);
    let want_cases = config.cases as usize;
    assert!(want_cases > 0, "THRUST_MODEL_CASES=0 would check nothing");
    let counts = Counts::default();
    let settle = Mutex::new(Vec::new());
    let mut runner = TestRunner::new(config);
    let result = runner.run(&ops(), |ops| rt.block_on(run_case(&t, &ops, fastpath, &counts, &settle)));
    let (cases, requests) = (counts.cases.load(Ordering::Relaxed), counts.requests.load(Ordering::Relaxed));
    eprintln!("MODEL-OBSERVATION: {cases} generated sequences, {requests} steps checked against the model");
    if let Err(e) = result {
        panic!("{e}");
    }
    assert!(cases >= want_cases && requests >= cases, "the generated sequences did not run: {cases} cases, {requests} steps");

    // A redelivery that lands after a checkpoint matched would slip past it: wait once for the whole run, then
    // read every case's activity again.
    std::thread::sleep(Duration::from_secs(2));
    let settle = settle.into_inner().unwrap();
    let late: Vec<String> = rt.block_on(async {
        let mut late = Vec::new();
        for s in &settle {
            let url = t.url(&format!("{}/activity", project_path(&s.at)));
            let rows: Vec<Activity> = t.http.get(url).bearer_auth(&s.token).send().await.unwrap().json().await.unwrap();
            let mut got: Vec<EventId> = rows.iter().map(|r| r.event_id).collect();
            got.sort();
            if got != s.event_ids {
                late.push(format!("project {}: event ids {got:?}, the model {:?}", s.at.project, s.event_ids));
            }
        }
        late
    });
    assert!(late.is_empty(), "activity changed after the run settled:\n{}", late.join("\n"));
}

#[test]
fn model_work_items_over_http_match_the_reference_model() {
    run_model(true, 16);
}

/// Only run in the `sweeper` pass, against a target started with the outbox fast path OFF, one test at a time (it
/// reads the global outbox depth).
#[test]
fn sweeper_model_outbox_delivers_what_was_swept_and_nothing_more() {
    run_model(false, 8);
}

/// Rule 8 for the two tests above: a sequence generator that never produces, say, a delete of an already deleted row
/// or a redelivery of a real outbox event would pass on a server that gets that case wrong. Samples sequences with a
/// fixed seed in both outbox modes (2000 each), replays them through the model alone, and requires every step class at
/// least once.
#[test]
fn model_strategy_reaches_every_step_class() {
    let mut seen = BTreeSet::new();
    for fastpath in [true, false] {
        let mut runner = TestRunner::deterministic();
        for _ in 0..2000 {
            let ops = ops().new_tree(&mut runner).unwrap().current();
            let mut m = Model::new(fastpath);
            for (step, op) in ops.iter().enumerate() {
                let plan = m.plan(step, op);
                seen.insert(plan.class);
                m.simulate(step, &plan);
            }
        }
    }
    let missing: Vec<_> = Class::ALL.iter().filter(|c| !seen.contains(c)).collect();
    assert!(missing.is_empty(), "never generated: {missing:?}");
}
