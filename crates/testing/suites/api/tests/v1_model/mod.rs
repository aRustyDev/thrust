//! Generated scenarios over HTTP against `/api/v1`'s work items (gap G-8 in `docs/src/testing/status.md`), part of the portable suite
//! and run on every target through the same `THRUST_BASE_URL` client, with an API key per caller:
//!
//! * `model_v1_work_items_over_http_match_the_reference_model` (`main` pass, outbox fast path on): random sequences of create, delete,
//!   detail and list by every kind of caller, in two tenants and a project that does not exist, checked against a reference model;
//! * `sweeper_model_v1_outbox_holds_one_event_per_change_and_nothing_more` (`sweeper` pass, fast path off): the same sequences, where
//!   every event waits in the outbox until a sweep, so the outbox depth shows what a request wrote;
//! * `model_v1_strategy_reaches_every_step_class`: the generator coverage control for both (Rule 8). It replays sampled sequences
//!   through the model alone and needs no target;
//! * `matrix::every_actor_project_and_route_gets_the_status_of_the_v1_access_table`: every caller × slug × project × route.
//!
//! The model is the spec, written out; it shares no code with the server. It is the pin's, as `thrust-api-v1`'s own tests and the
//! oracle record it (D-033), not the slice's:
//!
//! * a missing key is `401` before anything else, and a key that is not valid is `403` `Given API token is not valid`;
//! * a project the caller is not a member of, one that is in another workspace than the slug names, one that does not exist and a
//!   slug that names no workspace are all the same `403` (`ProjectEntityPermission`), and so is a guest who writes: create and delete
//!   need Member, detail and list need Guest. The permission is checked before the body, so a guest's bad body is a `403`, not a `400`;
//! * a create with a bad name (blank, or over 255 characters after trimming) is `400` in DRF's words; a live
//!   `(external_source, external_id)` is unique per project (`409` naming the work item that has it), and a soft delete frees it;
//! * sequence ids per project start at 1, strictly increase and are never reused. A create that fails after the number was allocated
//!   (a `409`) may leave a gap, at most one per failed create: ADR-0043 accepts gaps on failure, and the adapters differ here;
//! * `sort_order` is 65535 for the first work item and 10000 after the greatest one of the live work items for each after it: the
//!   pin reads it before it inserts, so after a deletion a new item can take a place a deleted one had;
//! * a detail is `200` for a live work item of the path's project and `404` for a deleted one, an unknown id or another project's;
//!   a delete is `204` for the same, `403` for a member who did not make it (a project admin deletes any), and `404` otherwise; a
//!   rejected request changes nothing (every later list checks that);
//! * a list is the live work items of that project, newest first (the greater number first, as the work items of a model run are
//!   made one after the other), and nothing outside the requested project (R-S1). A page is `per_page` of them, the `cursor`
//!   `per_page:page:0` asks for the next, `total_count` is all of them and `next_page_results` says whether there is another page;
//! * events (ADR-0046): every successful create and delete puts exactly one event in the outbox, and a rejected or conflicting request
//!   puts none. With the fast path on the event is published at once; with it off it waits until a sweep, which publishes every
//!   waiting row (`found` = `published` = the outbox depth, nothing `failed`), and the outbox depth is always the number of waiting
//!   events. (`/api/v1` has no activity feed, so what the consumer does with an event is the slice model's to check, for now.)
//!
//! Budget: `THRUST_MODEL_CASES` sequences per run (default 16 in `main`, 8 in `sweeper`), deliberately not `PROPTEST_CASES`, so a `deep`
//! run's setting cannot turn one contract cell into thousands of round trips. Each case seeds its own fixture, so it starts from empty
//! projects and fresh keys. A key may make 60 requests a minute (Plane's default, the `main` pass's `/api/v1` limit), and a sequence of
//! `MAX_OPS` operations makes at most two requests each, plus the two reads that end it, so no sequence reaches the limit and the model
//! never has to predict a 429.

mod matrix;

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

const MAX_OPS: usize = 28;

/// The texts the pin answers with (`thrust-api-v1`'s `error.rs`, the oracle's corpus).
const AUTH_MISSING: &str = "Authentication credentials were not provided.";
const KEY_NOT_VALID: &str = "Given API token is not valid";
const DENIED: &str = "You do not have permission to perform this action.";
const NOT_FOUND: &str = "The requested resource does not exist.";
const ONLY_ADMIN_OR_MAKER: &str = "Only admin or creator can delete the work item";
const EXTERNAL_KEY_TAKEN: &str = "Issue with the same external id and external source already exists";

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
    /// No `X-Api-Key` header.
    Anonymous,
    /// A key that is not one.
    BadKey,
}

/// Every actor. A new variant fails to compile in `standing` until the table says what it may do; add it here too.
const ALL_ACTORS: [Actor; 7] =
    [Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA, Actor::AdminB, Actor::Anonymous, Actor::BadKey];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Proj {
    A,
    B,
    /// A project id that was never seeded.
    Unknown,
}

const ALL_PROJECTS: [Proj; 3] = [Proj::A, Proj::B, Proj::Unknown];

/// Which workspace slug a request names, relative to the project it addresses. The generated sequences name the project's own
/// workspace nearly always and one of the other two now and then; `matrix` covers all three for every actor, project and route.
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
    NoKey,
    BadKey,
    NoSuchProject,
    NotAMember,
    Member(Role),
}

/// What `actor` is in `proj`, from the fixture's memberships (`Fixture::seed`), written out rather than asked of the server.
fn standing(actor: Actor, proj: Proj) -> Standing {
    match (actor, proj) {
        (Actor::Anonymous, _) => Standing::NoKey,
        (Actor::BadKey, _) => Standing::BadKey,
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

/// A refusal as the pin words it: a status, the key of the body that carries the text, and the text.
#[derive(Debug, Clone, PartialEq)]
struct Refusal {
    status: u16,
    body: Value,
}

impl Refusal {
    fn detail(status: u16, text: &str) -> Refusal {
        Refusal { status, body: json!({ "detail": text }) }
    }
}

/// The rejection a request needing `min` gets from authentication and the project's permission, or `None` when it passes. Authentication
/// comes first (a missing key is a 401 and a key that is not valid a 403). Then the slug: one that is not the project's own workspace's
/// names another workspace, or none, and the project is not in the workspace it names, which is refused to whoever asks. Then the
/// project: one that is not the caller's (another tenant's, one that is not there, one the caller is no member of) and a role that is too
/// low are one answer, the pin's; the class only says which of them a step was, for the coverage control.
fn gate(actor: Actor, proj: Proj, slug: Slug, min: Role) -> Option<(Refusal, Class)> {
    let denied = |class| Some((Refusal::detail(403, DENIED), class));
    match (standing(actor, proj), slug) {
        (Standing::NoKey, _) => Some((Refusal::detail(401, AUTH_MISSING), Class::NoKey)),
        (Standing::BadKey, _) => Some((Refusal::detail(403, KEY_NOT_VALID), Class::BadKey)),
        (_, Slug::Other | Slug::Unknown) => denied(Class::WrongSlug),
        (Standing::NoSuchProject, Slug::Own) if proj == Proj::Unknown => denied(Class::NoSuchProject),
        (Standing::NoSuchProject, Slug::Own) => denied(Class::CrossTenant),
        (Standing::NotAMember, Slug::Own) => denied(Class::NotAMember),
        (Standing::Member(r), Slug::Own) if r >= min => None,
        (Standing::Member(_), Slug::Own) => denied(Class::RoleTooLow),
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
enum Pick {
    /// One of the path project's items, live or deleted (an unknown id when it has none).
    Own(usize),
    /// One of the other seeded project's items: a detail or a delete must not reach it through this path.
    OtherProject(usize),
    Unknown,
}

/// How a list is asked for.
#[derive(Debug, Clone, Copy)]
enum Page {
    /// With no `per_page`: the pin's default is a thousand, which holds the whole list.
    Whole,
    /// `per_page` of this many (1 to 3), the first page and, when there is one, the second (by the cursor the answer names).
    Of(u8),
}

#[derive(Debug, Clone)]
enum Op {
    /// `ext`: an external key from a small domain, so that keys collide.
    Create {
        actor: Actor,
        proj: Proj,
        slug: Slug,
        name: Name,
        ext: Option<u8>,
    },
    Delete {
        actor: Actor,
        proj: Proj,
        slug: Slug,
        pick: Pick,
    },
    Get {
        actor: Actor,
        proj: Proj,
        slug: Slug,
        pick: Pick,
    },
    List {
        actor: Actor,
        proj: Proj,
        slug: Slug,
        page: Page,
    },
    /// Run the outbox sweeper (`/internal/sweep`), what cron does.
    Sweep,
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
        1 => Just(Actor::BadKey),
    ]
}

fn proj() -> impl Strategy<Value = Proj> {
    prop_oneof![5 => Just(Proj::A), 3 => Just(Proj::B), 1 => Just(Proj::Unknown)]
}

/// The slug a request names: nearly always the project's own workspace's, so that sequences build up state worth checking, and
/// sometimes another workspace's or none's, which the pin refuses whoever asks (and which `ignore-tenant`, a server that trusts the
/// project's own workspace over the slug, gets wrong).
fn slug() -> impl Strategy<Value = Slug> {
    prop_oneof![8 => Just(Slug::Own), 1 => Just(Slug::Other), 1 => Just(Slug::Unknown)]
}

/// A project, a slug and a caller: usually one who may write there, so sequences build up state worth checking, otherwise anyone (drawn
/// independently, the pair is mostly a cross-tenant or unauthenticated rejection).
fn caller() -> impl Strategy<Value = (Actor, Proj, Slug)> {
    proj().prop_flat_map(|p| {
        let writers = match p {
            Proj::A => vec![Actor::AdminA, Actor::MemberA],
            Proj::B | Proj::Unknown => vec![Actor::AdminB],
        };
        (prop_oneof![3 => prop::sample::select(writers), 2 => actor()], Just(p), slug())
    })
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

fn pick() -> impl Strategy<Value = Pick> {
    prop_oneof![6 => (0..16usize).prop_map(Pick::Own), 2 => (0..16usize).prop_map(Pick::OtherProject), 1 => Just(Pick::Unknown)]
}

fn page() -> impl Strategy<Value = Page> {
    prop_oneof![2 => Just(Page::Whole), 3 => (1..=3u8).prop_map(Page::Of)]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (caller(), name(), prop::option::weighted(0.4, 0..3u8))
            .prop_map(|((actor, proj, slug), name, ext)| Op::Create { actor, proj, slug, name, ext }),
        4 => (caller(), pick()).prop_map(|((actor, proj, slug), pick)| Op::Delete { actor, proj, slug, pick }),
        2 => (caller(), pick()).prop_map(|((actor, proj, slug), pick)| Op::Get { actor, proj, slug, pick }),
        3 => (caller(), page()).prop_map(|((actor, proj, slug), page)| Op::List { actor, proj, slug, page }),
        1 => Just(Op::Sweep),
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
    NoKey,
    BadKey,
    /// A slug that is not the project's own workspace's: another workspace's, or none's.
    WrongSlug,
    CrossTenant,
    NoSuchProject,
    NotAMember,
    RoleTooLow,
    InvalidName,
    Created,
    /// A create after a `409` in the same project: the place where a sequence gap may appear.
    CreatedAfterFailure,
    /// A create reusing the external key of a soft-deleted row.
    ExternalKeyFreedByDelete,
    /// The first create after a deletion, which takes the place the greatest live work item leaves.
    CreatedAfterDelete,
    Conflict,
    Deleted,
    /// A project admin deleting what a member made.
    DeletedByAdmin,
    /// A member deleting what the admin made.
    NotTheMaker,
    DeleteAlreadyDeleted,
    DeleteOtherProjectsItem,
    DeleteUnknownId,
    Read,
    ReadDeleted,
    ReadOtherProjectsItem,
    ReadUnknownId,
    Listed,
    ListedPage,
    ListedSecondPage,
    /// A sweep with nothing waiting (always, with the fast path on).
    Swept,
    /// A sweep that publishes waiting events (fast path off only).
    SweptPending,
}

impl Class {
    const ALL: [Class; 28] = [
        Class::NoKey,
        Class::BadKey,
        Class::WrongSlug,
        Class::CrossTenant,
        Class::NoSuchProject,
        Class::NotAMember,
        Class::RoleTooLow,
        Class::InvalidName,
        Class::Created,
        Class::CreatedAfterFailure,
        Class::ExternalKeyFreedByDelete,
        Class::CreatedAfterDelete,
        Class::Conflict,
        Class::Deleted,
        Class::DeletedByAdmin,
        Class::NotTheMaker,
        Class::DeleteAlreadyDeleted,
        Class::DeleteOtherProjectsItem,
        Class::DeleteUnknownId,
        Class::Read,
        Class::ReadDeleted,
        Class::ReadOtherProjectsItem,
        Class::ReadUnknownId,
        Class::Listed,
        Class::ListedPage,
        Class::ListedSecondPage,
        Class::Swept,
        Class::SweptPending,
    ];
}

#[derive(Debug, Clone)]
struct Row {
    id: String,
    seq: i64,
    name: String,
    ext: Option<u8>,
    by: Actor,
    sort: i64,
    deleted: bool,
}

#[derive(Debug, Clone, Default)]
struct ProjectModel {
    rows: Vec<Row>,
    last_seq: i64,
    /// `409`s since the last successful create: each may have used up a sequence number.
    failed_since: i64,
    /// Whether a work item was deleted since the last successful create (the class of the create that follows).
    deleted_since: bool,
}

impl ProjectModel {
    /// The place a work item made now gets: 65535 for the first, 10000 after the greatest of the live ones for each other.
    fn next_sort(&self) -> i64 {
        self.rows.iter().filter(|r| !r.deleted).map(|r| r.sort).max().map_or(65_535, |greatest| greatest + 10_000)
    }

    fn live_with_key(&self, k: u8) -> Option<&Row> {
        self.rows.iter().find(|r| !r.deleted && r.ext == Some(k))
    }

    fn key_was_freed(&self, k: u8) -> bool {
        self.rows.iter().any(|r| r.deleted && r.ext == Some(k))
    }

    /// The rows a list must show, newest first.
    fn listed(&self) -> Vec<&Row> {
        let mut live: Vec<&Row> = self.rows.iter().filter(|r| !r.deleted).collect();
        live.sort_by_key(|r| std::cmp::Reverse(r.seq));
        live
    }
}

/// Whose item a request names.
#[derive(Debug, Clone, Copy)]
enum ItemRef {
    /// A row of the path's project, by index.
    Own(usize),
    /// An item of the other seeded project.
    Other,
    Unknown,
}

#[derive(Debug)]
struct Model {
    projects: [ProjectModel; 2],
    /// Whether the target publishes events right after commit (`main` pass) or leaves them to the sweeper.
    fastpath: bool,
    /// Events written and not yet swept: a create or a delete that succeeded.
    pending: usize,
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
    /// A rejection, checked on its status and its body.
    Reject(Refusal),
    Conflict {
        slot: usize,
        /// The id of the live work item that has the key.
        existing: String,
    },
    Create {
        slot: usize,
        name: String,
        ext: Option<u8>,
        actor: Actor,
        sort: i64,
    },
    Delete {
        slot: usize,
        row: usize,
    },
    Get {
        slot: usize,
        row: usize,
    },
    List {
        slot: usize,
        page: Page,
    },
    Sweep,
}

/// One work-item request, in model terms; `World::send` turns it into ids.
#[derive(Debug, Clone)]
struct Req {
    actor: Actor,
    method: Method,
    proj: Proj,
    slug: Slug,
    item: Option<String>,
    query: Vec<(&'static str, String)>,
    body: Option<Value>,
}

impl Req {
    fn bare(actor: Actor, method: Method, proj: Proj) -> Req {
        Req { actor, method, proj, slug: Slug::Own, item: None, query: Vec::new(), body: None }
    }
}

/// A response, as far as the model looks at it.
struct Resp {
    status: u16,
    body: Value,
    text: String,
}

impl Resp {
    /// The body for a failure message, cut short.
    fn shown(&self) -> String {
        self.text.chars().take(300).collect()
    }
}

#[derive(Debug, Clone)]
enum Call {
    Api(Req),
    Sweep,
}

struct Plan {
    call: Call,
    want: Want,
    class: Class,
}

fn unknown_item(step: usize) -> String {
    Uuid::from_u128(0xBAD0_0000 + step as u128).to_string()
}

impl Model {
    fn new(fastpath: bool) -> Model {
        Model { projects: Default::default(), fastpath, pending: 0 }
    }

    fn plan(&self, step: usize, op: &Op) -> Plan {
        match *op {
            Op::Create { actor, proj, slug, name, ext } => {
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
                let call = Call::Api(Req { body: Some(body), slug, ..Req::bare(actor, Method::POST, proj) });
                let (want, class) = if let Some((refusal, class)) = gate(actor, proj, slug, Role::Member) {
                    (Want::Reject(refusal), class)
                } else if matches!(name, Name::Blank) {
                    (
                        Want::Reject(Refusal {
                            status: 400,
                            body: json!({ "name": ["This field may not be blank."] }),
                        }),
                        Class::InvalidName,
                    )
                } else if matches!(name, Name::TooLong) {
                    let said = json!({ "name": ["Ensure this field has no more than 255 characters."] });
                    (Want::Reject(Refusal { status: 400, body: said }), Class::InvalidName)
                } else {
                    let s = slot(proj).expect("an authorized project is a seeded one");
                    let p = &self.projects[s];
                    match ext.and_then(|k| p.live_with_key(k).map(|row| (k, row))) {
                        Some((_, row)) => (Want::Conflict { slot: s, existing: row.id.clone() }, Class::Conflict),
                        None => {
                            let class = if p.failed_since > 0 {
                                Class::CreatedAfterFailure
                            } else if ext.is_some_and(|k| p.key_was_freed(k)) {
                                Class::ExternalKeyFreedByDelete
                            } else if p.deleted_since {
                                Class::CreatedAfterDelete
                            } else {
                                Class::Created
                            };
                            (
                                Want::Create { slot: s, name: raw.trim().to_string(), ext, actor, sort: p.next_sort() },
                                class,
                            )
                        }
                    }
                };
                Plan { call, want, class }
            }
            Op::Delete { actor, proj, slug, pick } => {
                let (item, target) = self.resolve(step, proj, pick);
                let call = Call::Api(Req { item: Some(item), slug, ..Req::bare(actor, Method::DELETE, proj) });
                let (want, class) = if let Some((refusal, class)) = gate(actor, proj, slug, Role::Member) {
                    (Want::Reject(refusal), class)
                } else {
                    let s = slot(proj).expect("an authorized project is a seeded one");
                    match target {
                        ItemRef::Own(r) if !self.projects[s].rows[r].deleted => {
                            let row = &self.projects[s].rows[r];
                            if actor == Actor::AdminB || actor == Actor::AdminA || row.by == actor {
                                let class = if row.by == actor { Class::Deleted } else { Class::DeletedByAdmin };
                                (Want::Delete { slot: s, row: r }, class)
                            } else {
                                let said = Refusal { status: 403, body: json!({ "error": ONLY_ADMIN_OR_MAKER }) };
                                (Want::Reject(said), Class::NotTheMaker)
                            }
                        }
                        ItemRef::Own(_) => (Want::Reject(not_found()), Class::DeleteAlreadyDeleted),
                        ItemRef::Other => (Want::Reject(not_found()), Class::DeleteOtherProjectsItem),
                        ItemRef::Unknown => (Want::Reject(not_found()), Class::DeleteUnknownId),
                    }
                };
                Plan { call, want, class }
            }
            Op::Get { actor, proj, slug, pick } => {
                let (item, target) = self.resolve(step, proj, pick);
                let call = Call::Api(Req { item: Some(item), slug, ..Req::bare(actor, Method::GET, proj) });
                let (want, class) = if let Some((refusal, class)) = gate(actor, proj, slug, Role::Guest) {
                    (Want::Reject(refusal), class)
                } else {
                    let s = slot(proj).expect("an authorized project is a seeded one");
                    match target {
                        ItemRef::Own(r) if !self.projects[s].rows[r].deleted => {
                            (Want::Get { slot: s, row: r }, Class::Read)
                        }
                        ItemRef::Own(_) => (Want::Reject(not_found()), Class::ReadDeleted),
                        ItemRef::Other => (Want::Reject(not_found()), Class::ReadOtherProjectsItem),
                        ItemRef::Unknown => (Want::Reject(not_found()), Class::ReadUnknownId),
                    }
                };
                Plan { call, want, class }
            }
            Op::List { actor, proj, slug, page } => {
                let query = match page {
                    Page::Whole => Vec::new(),
                    Page::Of(n) => vec![("per_page", n.to_string())],
                };
                let call = Call::Api(Req { query, slug, ..Req::bare(actor, Method::GET, proj) });
                let (want, class) = if let Some((refusal, class)) = gate(actor, proj, slug, Role::Guest) {
                    (Want::Reject(refusal), class)
                } else {
                    let s = slot(proj).expect("an authorized project is a seeded one");
                    let class = match page {
                        Page::Whole => Class::Listed,
                        Page::Of(n) if self.projects[s].listed().len() > usize::from(n) => Class::ListedSecondPage,
                        Page::Of(_) => Class::ListedPage,
                    };
                    (Want::List { slot: s, page }, class)
                };
                Plan { call, want, class }
            }
            Op::Sweep => {
                let class = if self.pending > 0 { Class::SweptPending } else { Class::Swept };
                Plan { call: Call::Sweep, want: Want::Sweep, class }
            }
        }
    }

    /// The item `pick` names in a request on `proj`: its id, and whose row it is.
    fn resolve(&self, step: usize, proj: Proj, pick: Pick) -> (String, ItemRef) {
        let other = match proj {
            Proj::A | Proj::Unknown => 1,
            Proj::B => 0,
        };
        let rows = |s: usize| &self.projects[s].rows;
        match pick {
            Pick::Own(i) => match slot(proj) {
                Some(s) if !rows(s).is_empty() => {
                    let r = i % rows(s).len();
                    (rows(s)[r].id.clone(), ItemRef::Own(r))
                }
                _ => (unknown_item(step), ItemRef::Unknown),
            },
            Pick::OtherProject(i) if !rows(other).is_empty() => {
                (rows(other)[i % rows(other).len()].id.clone(), ItemRef::Other)
            }
            Pick::OtherProject(_) | Pick::Unknown => (unknown_item(step), ItemRef::Unknown),
        }
    }

    fn record_conflict(&mut self, slot: usize) {
        self.projects[slot].failed_since += 1;
    }

    fn record_create(&mut self, slot: usize, row: Row) {
        let p = &mut self.projects[slot];
        p.last_seq = row.seq;
        p.failed_since = 0;
        p.deleted_since = false;
        p.rows.push(row);
        self.emit();
    }

    fn record_delete(&mut self, slot: usize, row: usize) {
        let p = &mut self.projects[slot];
        p.rows[row].deleted = true;
        p.deleted_since = true;
        self.emit();
    }

    /// The outbox row a committed create or delete writes: published at once with the fast path on, and waiting for a sweep with it off.
    fn emit(&mut self) {
        if !self.fastpath {
            self.pending += 1;
        }
    }

    /// Offline replay for the coverage control: apply the predicted effect with made-up ids.
    fn simulate(&mut self, step: usize, plan: &Plan) {
        match plan.want {
            Want::Reject(_) | Want::Get { .. } | Want::List { .. } => {}
            Want::Conflict { slot, .. } => self.record_conflict(slot),
            Want::Create { slot, ref name, ext, actor, sort } => {
                let seq = self.projects[slot].last_seq + 1;
                let id = Uuid::from_u128(step as u128 + 1).to_string();
                self.record_create(slot, Row { id, seq, name: name.clone(), ext, by: actor, sort, deleted: false });
            }
            Want::Delete { slot, row } => self.record_delete(slot, row),
            Want::Sweep => self.pending = 0,
        }
    }
}

fn not_found() -> Refusal {
    Refusal { status: 404, body: json!({ "error": NOT_FOUND }) }
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
            Actor::Anonymous | Actor::BadKey => None,
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
            Actor::BadKey => b.header(API_KEY_HEADER, "not-a-key"),
            a => b.header(API_KEY_HEADER, &self.user(a).expect("an authenticated actor has a user").key),
        }
    }

    async fn send(&self, req: &Req) -> Resp {
        self.try_send(req).await.unwrap()
    }

    async fn try_send(&self, req: &Req) -> Result<Resp, reqwest::Error> {
        let at = self.at(req.proj, req.slug);
        let path = match &req.item {
            Some(id) => v1_item(&at, id),
            None => v1_items(&at),
        };
        let mut b = self.as_actor(self.t.http.request(req.method.clone(), self.t.url(&path)), req.actor);
        if !req.query.is_empty() {
            b = b.query(&req.query);
        }
        if let Some(body) = &req.body {
            b = b.json(body);
        }
        let r = b.send().await?;
        let status = r.status().as_u16();
        let text = r.text().await.unwrap_or_default();
        let body = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(Resp { status, body, text })
    }
}

/// Sequences run and requests sent, shrinking included: the test fails if it compared nothing. The counts are printed as
/// `MODEL-OBSERVATION` (visible on failure, or with `--nocapture`).
#[derive(Default)]
struct Counts {
    cases: AtomicUsize,
    requests: AtomicUsize,
}

fn fail(at: &str, e: impl std::fmt::Display) -> TestCaseError {
    TestCaseError::fail(format!("{at}: {e}"))
}

/// What a list answered (a page of it): the numbers and names of its rows, newest first, after the envelope's own checks.
fn rows_of(body: &Value, scope: (WorkspaceId, ProjectId), at: &str) -> Result<Vec<(i64, String)>, TestCaseError> {
    let results = body["results"].as_array().ok_or_else(|| fail(at, format!("no `results` in {body}")))?;
    let mut rows = Vec::with_capacity(results.len());
    for item in results {
        let (workspace, project) = (scope.0 .0.to_string(), scope.1 .0.to_string());
        if item["workspace"].as_str() != Some(workspace.as_str()) || item["project"].as_str() != Some(project.as_str())
        {
            return Err(fail(at, format!("a row outside its scope: {item}")));
        }
        rows.push((item["sequence_id"].as_i64().unwrap_or(-1), item["name"].as_str().unwrap_or("?").to_string()));
    }
    Ok(rows)
}

async fn run_case(t: &Target, ops: &[Op], fastpath: bool, counts: &Counts) -> Result<(), TestCaseError> {
    counts.cases.fetch_add(1, Ordering::Relaxed);
    let w = World::seed(t).await;
    let mut m = Model::new(fastpath);
    // Undispatched rows other tests left behind; a sweep takes them too. Only read with the fast path off, where the pass runs one
    // test at a time.
    let mut leftover = if fastpath { 0 } else { t.outbox_depth().await };
    // Every case ends by reading both projects as their admins (so a request that changed what it should not have shows even when no
    // list was drawn), and, with the fast path off, a sweep first so that the outbox is empty again.
    let mut all: Vec<Op> = ops.to_vec();
    if !fastpath {
        all.push(Op::Sweep);
    }
    all.push(Op::List { actor: Actor::AdminA, proj: Proj::A, slug: Slug::Own, page: Page::Whole });
    all.push(Op::List { actor: Actor::AdminB, proj: Proj::B, slug: Slug::Own, page: Page::Whole });
    for (step, op) in all.iter().enumerate() {
        let plan = m.plan(step, op);
        counts.requests.fetch_add(1, Ordering::Relaxed);
        let at = format!("step {step}: {op:?} ({:?})", plan.class);
        match (plan.call, plan.want) {
            (Call::Api(req), want) => {
                let resp = w.try_send(&req).await.map_err(|e| fail(&at, e))?;
                let (status, shown) = (resp.status, resp.shown());
                match want {
                    Want::Reject(refusal) => {
                        prop_assert_eq!(status, refusal.status, "{}; body {}", at, shown);
                        prop_assert_eq!(&resp.body, &refusal.body, "{}: the pin's words", at);
                    }
                    Want::Conflict { slot, existing } => {
                        prop_assert_eq!(status, 409, "{}; body {}", at, shown);
                        prop_assert_eq!(
                            &resp.body,
                            &json!({ "error": EXTERNAL_KEY_TAKEN, "id": existing }),
                            "{}: the 409 names the work item that has the key",
                            at
                        );
                        m.record_conflict(slot);
                    }
                    Want::Create { slot, name, ext, actor, sort } => {
                        prop_assert_eq!(status, 201, "{}; body {}", at, shown);
                        let b = &resp.body;
                        let (workspace, project) = w.scope_of(slot);
                        let (workspace, project) = (workspace.0.to_string(), project.0.to_string());
                        prop_assert_eq!(
                            (b["workspace"].as_str(), b["project"].as_str()),
                            (Some(workspace.as_str()), Some(project.as_str())),
                            "{}: created outside its scope",
                            at
                        );
                        prop_assert_eq!(
                            b["name"].as_str(),
                            Some(name.as_str()),
                            "{}: the stored name is the trimmed name",
                            at
                        );
                        let maker = w.user(actor).map(|u| u.id.0.to_string());
                        prop_assert_eq!(b["created_by"].as_str(), maker.as_deref(), "{}", at);
                        prop_assert!(b["deleted_at"].is_null(), "{}: a new item is live", at);
                        let key = ext.map(|k| format!("k{k}"));
                        prop_assert_eq!(
                            (b["external_source"].as_str(), b["external_id"].as_str()),
                            (ext.map(|_| "gh"), key.as_deref()),
                            "{}: the integration key is kept as given",
                            at
                        );
                        let id = b["id"].as_str().ok_or_else(|| fail(&at, "no id"))?.to_string();
                        let seq = b["sequence_id"].as_i64().ok_or_else(|| fail(&at, "no sequence_id"))?;
                        let p = &m.projects[slot];
                        prop_assert!(
                            seq > p.last_seq && seq <= p.last_seq + 1 + p.failed_since,
                            "{}: sequence id {} after {} with {} failed create(s) since: must increase, never repeat, and skip at \
                             most one number per failed create",
                            at,
                            seq,
                            p.last_seq,
                            p.failed_since
                        );
                        prop_assert_eq!(
                            b["sort_order"].as_f64(),
                            Some(sort as f64),
                            "{}: the pin's place in the state",
                            at
                        );
                        m.record_create(slot, Row { id, seq, name, ext, by: actor, sort, deleted: false });
                    }
                    Want::Delete { slot, row } => {
                        prop_assert_eq!(status, 204, "{}; body {}", at, shown);
                        m.record_delete(slot, row);
                    }
                    Want::Get { slot, row } => {
                        prop_assert_eq!(status, 200, "{}; body {}", at, shown);
                        let r = &m.projects[slot].rows[row];
                        prop_assert_eq!(
                            (resp.body["id"].as_str(), resp.body["sequence_id"].as_i64(), resp.body["name"].as_str()),
                            (Some(r.id.as_str()), Some(r.seq), Some(r.name.as_str())),
                            "{}",
                            at
                        );
                        let project = w.scope_of(slot).1 .0.to_string();
                        prop_assert_eq!(
                            resp.body["project"].as_str(),
                            Some(project.as_str()),
                            "{}: a work item outside its scope",
                            at
                        );
                    }
                    Want::List { slot, page } => {
                        prop_assert_eq!(status, 200, "{}; body {}", at, shown);
                        let scope = w.scope_of(slot);
                        let want_rows: Vec<(i64, String)> =
                            m.projects[slot].listed().iter().map(|r| (r.seq, r.name.clone())).collect();
                        let total = want_rows.len();
                        prop_assert_eq!(resp.body["total_count"].as_u64(), Some(total as u64), "{}: total_count", at);
                        match page {
                            Page::Whole => {
                                prop_assert_eq!(rows_of(&resp.body, scope, &at)?, want_rows, "{}", at);
                                prop_assert_eq!(resp.body["next_page_results"].as_bool(), Some(false), "{}", at);
                            }
                            Page::Of(n) => {
                                let n = usize::from(n);
                                prop_assert_eq!(
                                    rows_of(&resp.body, scope, &at)?,
                                    want_rows[..total.min(n)].to_vec(),
                                    "{}",
                                    at
                                );
                                prop_assert_eq!(resp.body["next_page_results"].as_bool(), Some(total > n), "{}", at);
                                if total > n {
                                    // the second page, by the cursor the answer names
                                    let cursor =
                                        resp.body["next_cursor"].as_str().ok_or_else(|| fail(&at, "no next_cursor"))?;
                                    prop_assert_eq!(cursor, format!("{n}:1:0"), "{}: the next page's cursor", at);
                                    counts.requests.fetch_add(1, Ordering::Relaxed);
                                    let next = Req {
                                        query: vec![("per_page", n.to_string()), ("cursor", cursor.to_string())],
                                        ..req.clone()
                                    };
                                    let second = w.send(&next).await;
                                    prop_assert_eq!(second.status, 200, "{}: second page; body {}", at, second.shown());
                                    prop_assert_eq!(
                                        rows_of(&second.body, scope, &at)?,
                                        want_rows[n..total.min(2 * n)].to_vec(),
                                        "{}: second page",
                                        at
                                    );
                                    prop_assert_eq!(
                                        second.body["next_page_results"].as_bool(),
                                        Some(total > 2 * n),
                                        "{}",
                                        at
                                    );
                                }
                            }
                        }
                    }
                    Want::Sweep => unreachable!("a sweep is not a work-item request"),
                }
                if !fastpath {
                    // what the request wrote: one event for a create or a delete that succeeded, none for a refusal or a conflict
                    let depth = t.outbox_depth().await;
                    prop_assert_eq!(
                        depth,
                        leftover + m.pending as u64,
                        "{}: outbox depth is the number of waiting events",
                        at
                    );
                }
            }
            (Call::Sweep, Want::Sweep) => {
                let r = t.admin_post("/internal/sweep", &json!({})).await;
                let status = r.status().as_u16();
                let body = r.text().await.unwrap_or_default();
                prop_assert_eq!(status, 200, "{}; body {}", at, body);
                if !fastpath {
                    let rep: Value = serde_json::from_str(&body).map_err(|e| fail(&at, e))?;
                    let waiting = leftover + m.pending as u64;
                    let got = (rep["found"].as_u64(), rep["published"].as_u64(), rep["failed"].as_u64());
                    prop_assert_eq!(
                        got,
                        (Some(waiting), Some(waiting), Some(0)),
                        "{}: the sweep must publish every waiting row",
                        at
                    );
                    leftover = 0;
                }
                m.pending = 0;
            }
            (call, want) => unreachable!("planned {call:?} with {want:?}"),
        }
    }
    Ok(())
}

/// Proptest config for this file: the HTTP case budget, a bounded shrink (each shrink step is a fresh run against the target, so the
/// shrink also stops after a minute), and counterexamples saved beside this file (`mod.proptest-regressions`, committed per Rule 7).
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
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let config = config(default_cases);
    let want_cases = config.cases as usize;
    assert!(want_cases > 0, "THRUST_MODEL_CASES=0 would check nothing");
    let counts = Counts::default();
    let mut runner = TestRunner::new(config);
    let result = runner.run(&ops(), |ops| rt.block_on(run_case(&t, &ops, fastpath, &counts)));
    let (cases, requests) = (counts.cases.load(Ordering::Relaxed), counts.requests.load(Ordering::Relaxed));
    eprintln!("MODEL-OBSERVATION: {cases} generated sequences, {requests} requests checked against the model");
    if let Err(e) = result {
        panic!("{e}");
    }
    assert!(
        cases >= want_cases && requests >= cases,
        "the generated sequences did not run: {cases} cases, {requests} steps"
    );
}

#[test]
fn model_v1_work_items_over_http_match_the_reference_model() {
    run_model(true, 16);
}

/// Only run in the `sweeper` pass, against a target started with the outbox fast path OFF, one test at a time (it reads the global
/// outbox depth).
#[test]
fn sweeper_model_v1_outbox_holds_one_event_per_change_and_nothing_more() {
    run_model(false, 8);
}

/// Rule 8 for the two tests above: a sequence generator that never produces, say, a delete of an already deleted row or a create
/// that follows a conflict would pass on a server that gets that case wrong. Samples sequences with a fixed seed in both outbox modes
/// (2000 each), replays them through the model alone, and requires every step class at least once.
#[test]
fn model_v1_strategy_reaches_every_step_class() {
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
