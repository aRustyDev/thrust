//! Generated scenarios over HTTP (gap G-8 in `docs/src/testing/status.md`), part of the portable suite and run on
//! every target through the same `THRUST_BASE_URL` client:
//!
//! * `model_work_items_over_http_match_the_reference_model` (`main` pass, outbox fast path on): random sequences of
//!   attachment puts, gets and deletes by every kind of caller, in two tenants and a project that does not exist,
//!   mixed with work items made and deleted through `/api/v1`, injected and redelivered events, sweeps and activity
//!   checkpoints, checked against a reference model;
//! * `sweeper_model_outbox_delivers_what_was_swept_and_nothing_more` (`sweeper` pass, fast path off): the same
//!   sequences, where every event waits in the outbox until a sweep;
//! * `model_strategy_reaches_every_step_class`: the generator coverage control for both (Rule 8). It replays sampled
//!   sequences through the model alone and needs no target;
//! * `matrix::every_actor_project_and_route_gets_the_status_of_the_access_table`: every caller × project × route.
//!
//! What this file is for is what no Plane route replaces: the attachment routes and the activity feed, and what the queue
//! consumer does with an event. The work items they are about are made and deleted through `/api/v1`, with an API key,
//! as the vehicle (the slice's own create, list, search and delete routes are no longer called by anything); that
//! route's own rules (names, external keys, sequence ids, the role table, the list) are the model of `v1_model`, and
//! are not repeated here. The model is the spec, written out; it shares no code with the server:
//!
//! * a missing or unknown bearer token is `401` before anything else;
//! * a project outside the caller's workspace, or one that does not exist, is `404`; a project the caller is not a
//!   member of is `403`; an attachment put or delete needs Member, a get and the activity feed need Guest;
//! * a work item made through `/api/v1` by a writer of the project is `201`, and its project's admin deletes it with
//!   `204`; deleting one that is already deleted, or an id that was never made, is `404`;
//! * events (ADR-0046): every successful create and delete puts exactly one event in the outbox, and a rejected
//!   request puts none. With the fast path on the event is published at once; with it off it waits
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
//! seeds its own fixture, so it starts from empty projects and fresh API keys (and so fresh rate-limit buckets);
//! `MAX_OPS` stays below a key's allowance on `/api/v1` (`v1_requests_are_rate_limited_per_key` holds that allowance,
//! which is Plane's, G-26) so the limiter is never reached, whoever makes every item.

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
    /// Make a work item through `/api/v1`, as a writer of the project (`maker`).
    Create {
        actor: Actor,
        proj: Proj,
    },
    /// Delete one through `/api/v1`, as the project's admin: its `pick`th row (live or already deleted), or an id that
    /// was never made when `None`.
    Delete {
        proj: Proj,
        pick: Option<usize>,
    },
    /// Put an envelope straight on the queue (`/internal/inject-event`), about one of the project's items.
    Inject {
        proj: Proj,
        item: usize,
        verb: Verb,
        replay: Replay,
    },
    /// Run the outbox sweeper (`/internal/sweep`), what cron does.
    Sweep,
    /// Read both projects' activity and compare it with the model.
    Checkpoint,
    PutAttachment {
        actor: Actor,
        proj: Proj,
        pick: Pick,
        name: BlobName,
        size: Size,
        seed: u8,
        ctype: Option<Ctype>,
    },
    GetAttachment {
        actor: Actor,
        proj: Proj,
        pick: Pick,
        name: BlobName,
    },
    DeleteAttachment {
        actor: Actor,
        proj: Proj,
        pick: Pick,
        name: BlobName,
    },
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

/// Who makes a work item, and where: a writer of a seeded project. Who else may, and what they are told, is `/api/v1`'s table
/// (`v1_model`), not this file's.
fn maker() -> impl Strategy<Value = (Actor, Proj)> {
    prop_oneof![
        2 => Just((Actor::AdminA, Proj::A)),
        3 => Just((Actor::MemberA, Proj::A)),
        3 => Just((Actor::AdminB, Proj::B)),
    ]
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
        5 => maker().prop_map(|(actor, proj)| Op::Create { actor, proj }),
        3 => (seeded(), prop::option::weighted(0.85, 0..16usize)).prop_map(|(proj, pick)| Op::Delete { proj, pick }),
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
    CrossTenant,
    NoSuchProject,
    NotAMember,
    RoleTooLow,
    Created,
    Deleted,
    DeleteAlreadyDeleted,
    DeleteUnknownId,
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
    const ALL: [Class; 29] = [
        Class::Unauthenticated,
        Class::CrossTenant,
        Class::NoSuchProject,
        Class::NotAMember,
        Class::RoleTooLow,
        Class::Created,
        Class::Deleted,
        Class::DeleteAlreadyDeleted,
        Class::DeleteUnknownId,
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
    deleted: bool,
}

#[derive(Debug, Clone, Default)]
struct ProjectModel {
    rows: Vec<Row>,
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
    Create {
        slot: usize,
        actor: Actor,
    },
    Delete {
        slot: usize,
        row: usize,
        actor: Actor,
    },
    /// `202`; the new event, or `None` for a redelivery (which must add nothing).
    Inject(Option<Event>),
    Sweep,
    Checkpoint,
    PutAttachment {
        slot: usize,
        key: (WorkItemId, String),
        stored: Stored,
    },
    GetAttachment(Stored),
    DeleteAttachment {
        slot: usize,
        key: (WorkItemId, String),
    },
}

/// One attachment request, in model terms; `World::send` turns it into ids. It goes through the slice's route
/// `.../work-items/<item>/attachments/<name>`, with a bearer token.
#[derive(Debug, Clone)]
struct Req {
    actor: Actor,
    method: Method,
    proj: Proj,
    slug: Slug,
    item: Option<WorkItemId>,
    /// `.../work-items/<item>/attachments/<name>`.
    attachment: Option<String>,
    /// The body and content type of an attachment put.
    upload: Option<Stored>,
}

impl Req {
    fn bare(actor: Actor, method: Method, proj: Proj) -> Req {
        Req { actor, method, proj, slug: Slug::Own, item: None, attachment: None, upload: None }
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
    /// An attachment request, through the slice's route.
    Api(Req),
    /// `POST /api/v1/.../issues/` with `actor`'s API key, into the project in `slot`.
    V1Create {
        actor: Actor,
        slot: usize,
        name: String,
    },
    /// `DELETE /api/v1/.../issues/{item}/` with `actor`'s API key, in the project in `slot`.
    V1Delete {
        actor: Actor,
        slot: usize,
        item: WorkItemId,
    },
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
            Op::Create { actor, proj } => {
                let s = slot(proj).expect("a work item is made in a seeded project");
                let call = Call::V1Create { actor, slot: s, name: format!("item {step}") };
                Plan { call, want: Want::Create { slot: s, actor }, class: Class::Created }
            }
            Op::Delete { proj, pick } => {
                let s = slot(proj).expect("a work item is deleted in a seeded project");
                let actor = admin_of(s);
                let rows = &self.projects[s].rows;
                let (item, want, class) = match pick {
                    Some(i) if !rows.is_empty() => {
                        let r = i % rows.len();
                        if rows[r].deleted {
                            (rows[r].id, Want::Reject(404), Class::DeleteAlreadyDeleted)
                        } else {
                            (rows[r].id, Want::Delete { slot: s, row: r, actor }, Class::Deleted)
                        }
                    }
                    _ => (unknown_item(step), Want::Reject(404), Class::DeleteUnknownId),
                };
                Plan { call: Call::V1Delete { actor, slot: s, item }, want, class }
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
                    Some((n, class)) => {
                        Plan { call: Call::Inject(self.events[n].clone()), want: Want::Inject(None), class }
                    }
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

    fn record_create(&mut self, slot: usize, row: Row, actor: Actor) {
        let entity = row.id.as_uuid();
        self.projects[slot].rows.push(row);
        self.emit(slot, entity, "created", actor);
    }

    fn record_delete(&mut self, slot: usize, row: usize, actor: Actor) {
        let p = &mut self.projects[slot];
        p.rows[row].deleted = true;
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
                    return Err(format!(
                        "event {} was recorded as {r:?}, the model sent {:?}",
                        r.event_id, self.events[n]
                    ))
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
            Want::Reject(_) => {}
            Want::Create { slot, actor } => {
                let id = WorkItemId(Uuid::from_u128(step as u128 + 1));
                self.record_create(slot, Row { id, deleted: false }, actor);
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
        let mut b = self.as_actor(self.t.http.request(req.method.clone(), self.t.url(&path)), req.actor);
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

    /// A request to `/api/v1`, with `actor`'s API key (the fixture plants one for each user). A transport failure is returned, not
    /// raised, so the case reports which step it was.
    async fn v1(
        &self,
        actor: Actor,
        method: Method,
        path: String,
        body: Option<Value>,
    ) -> Result<Resp, reqwest::Error> {
        let who = self.user(actor).expect("only a caller with a user makes a work item through /api/v1");
        let mut b = self.t.http.request(method, self.t.url(&path)).header(API_KEY_HEADER, &who.key);
        if let Some(body) = &body {
            b = b.json(body);
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
                    other => unreachable!("an attachment request planned {other:?}"),
                }
            }
            (Call::V1Create { actor, slot, name }, Want::Create { .. }) => {
                let path = v1_items(w.at_slot(slot));
                let resp =
                    w.v1(actor, Method::POST, path, Some(json!({ "name": name }))).await.map_err(|e| fail(&at, e))?;
                let (status, body) = (resp.status, resp.text());
                prop_assert_eq!(status, 201, "{}; body {}", at, body);
                let issue = Issue(serde_json::from_slice(&resp.body).map_err(|e| fail(&at, e))?);
                let (ws, project) = w.scope_of(slot);
                let project_id = project.0.to_string();
                prop_assert_eq!(issue.workspace(), Some(ws), "{}: made outside its workspace", at);
                prop_assert_eq!(
                    issue.0["project"].as_str(),
                    Some(project_id.as_str()),
                    "{}: made outside its project",
                    at
                );
                prop_assert_eq!(issue.created_by(), w.user(actor).map(|u| u.id), "{}", at);
                m.record_create(slot, Row { id: issue.work_item_id(), deleted: false }, actor);
            }
            (Call::V1Delete { actor, slot, item }, want) => {
                let path = v1_item(w.at_slot(slot), &item.to_string());
                let resp = w.v1(actor, Method::DELETE, path, None).await.map_err(|e| fail(&at, e))?;
                let (status, body) = (resp.status, resp.text());
                match want {
                    Want::Reject(want) => prop_assert_eq!(status, want, "{}; body {}", at, body),
                    Want::Delete { slot, row, actor } => {
                        prop_assert_eq!(status, 204, "{}; body {}", at, body);
                        m.record_delete(slot, row, actor);
                    }
                    other => unreachable!("a delete planned {other:?}"),
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
                    prop_assert_eq!(
                        got,
                        (Some(waiting), Some(waiting), Some(0)),
                        "{}: the sweep must publish every waiting row",
                        at
                    );
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
                            prop_assert_eq!(
                                (r.workspace_id, r.project_id),
                                w.scope_of(s),
                                "{}: activity outside its scope",
                                at
                            );
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
                    prop_assert_eq!(
                        depth,
                        leftover + m.pending() as u64,
                        "{}: outbox depth is the number of waiting events",
                        at
                    );
                }
            }
            (call, want) => unreachable!("planned {call:?} with {want:?}"),
        }
    }
    let mut late = settle.lock().unwrap();
    for s in 0..2 {
        let mut event_ids: Vec<EventId> = m
            .events
            .iter()
            .filter(|e| e.slot == s)
            .map(|e| e.id.expect("the last checkpoint matched every event"))
            .collect();
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

/// Sequences that were once counterexamples, run before the generated ones whatever the generator draws (a pinned
/// regression, Rule 9a).
///
/// The one saved in `mod.proptest-regressions` while this file drove the slice's own work-item routes: a bearer token that
/// is not a session puts a 5 MiB attachment (the server refuses it with a 401 before it reads the body, and may close the
/// connection while the body is still being sent), among attachment puts and deletes by callers of every kind. The saved
/// seed cannot replay against the strategy as it is now (the work items are made through `/api/v1`), so its attachment
/// steps are pinned here; its three other steps were deletes through the slice's own route, which is gone.
fn pinned() -> Vec<Vec<Op>> {
    vec![vec![
        Op::Checkpoint,
        Op::PutAttachment {
            actor: Actor::BadToken,
            proj: Proj::A,
            pick: Pick::Unknown,
            name: BlobName::Data,
            size: Size::OverLimit,
            seed: 52,
            ctype: None,
        },
        Op::PutAttachment {
            actor: Actor::AdminB,
            proj: Proj::Unknown,
            pick: Pick::Own(6),
            name: BlobName::Data,
            size: Size::Small(40),
            seed: 71,
            ctype: Some(Ctype::Custom),
        },
        Op::DeleteAttachment { actor: Actor::AdminA, proj: Proj::A, pick: Pick::Own(5), name: BlobName::DoubleDot },
        Op::DeleteAttachment { actor: Actor::AdminB, proj: Proj::B, pick: Pick::Own(11), name: BlobName::Data },
        Op::PutAttachment {
            actor: Actor::OutsiderA,
            proj: Proj::B,
            pick: Pick::Own(2),
            name: BlobName::Notes,
            size: Size::Small(60),
            seed: 103,
            ctype: None,
        },
        Op::Sweep,
    ]]
}

fn run_model(fastpath: bool, default_cases: u32) {
    let t = Target::from_env();
    assert!(
        MAX_OPS < Target::v1_limit() as usize,
        "a sequence must stay under an API key's allowance on /api/v1 ({}), or the model would need to predict 429s",
        Target::v1_limit()
    );
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let config = config(default_cases);
    let want_cases = config.cases as usize;
    assert!(want_cases > 0, "THRUST_MODEL_CASES=0 would check nothing");
    let counts = Counts::default();
    let settle = Mutex::new(Vec::new());
    let pinned = pinned();
    for ops in &pinned {
        if let Err(e) = rt.block_on(run_case(&t, ops, fastpath, &counts, &settle)) {
            panic!("a pinned sequence failed: {e}\n{ops:?}");
        }
    }
    let mut runner = TestRunner::new(config);
    let result = runner.run(&ops(), |ops| rt.block_on(run_case(&t, &ops, fastpath, &counts, &settle)));
    let (cases, requests) = (counts.cases.load(Ordering::Relaxed), counts.requests.load(Ordering::Relaxed));
    eprintln!(
        "MODEL-OBSERVATION: {cases} sequences ({} pinned), {requests} steps checked against the model",
        pinned.len()
    );
    if let Err(e) = result {
        panic!("{e}");
    }
    assert!(
        cases >= want_cases + pinned.len() && requests >= cases,
        "the generated sequences did not run: {cases} cases, {requests} steps"
    );

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
