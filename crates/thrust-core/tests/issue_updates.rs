//! Changing a work item and its history (issue #44, X03 and X02), against the memory adapters.
//!
//! What must hold: an admin or a member of the project changes any work item, a guest only one they made, anyone else none (and a
//! work item that is not there is `NotFound` only to those who may change one); a change applies what it names and leaves the rest,
//! moves `completed_at` only when the state moves, assigns only members of the project and refuses what it names that is not there
//! or a start after the target; and what it records of itself, which an activity consumer writes into the work item's history once
//! however often the event is delivered, is one row for each difference between what it asked and what the work item had, in the
//! order it asked, with a change to a description by the same person joined to the one before it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_adapters_memory::{ConsumerSlot, MemoryQueue};
use thrust_contracts::chrono::{DateTime, NaiveDate};
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

// --- the world -----------------------------------------------------------------------------------------------------------------

/// A clock that moves a second each time it is read, so a time that was set and one that was kept can be told apart.
struct Ticking(AtomicI64);

impl Clock for Ticking {
    fn now(&self) -> Timestamp {
        DateTime::from_timestamp(1_760_000_000 + self.0.fetch_add(1, Ordering::Relaxed), 0).unwrap()
    }
}

fn ticking_world() -> World {
    let background: Arc<dyn Background> = Arc::new(NoBackground);
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), ConsumerSlot::default()));
    seeded_in(world_built(
        CoreConfig { outbox_fastpath: false, ..CoreConfig::default() },
        queue,
        background,
        Arc::new(Ticking(AtomicI64::new(0))),
    ))
}

fn draft(name: &str) -> IssueDraft {
    IssueDraft { name: name.into(), ..IssueDraft::default() }
}

fn make(w: &World, who: Actor, d: IssueDraft) -> WorkItemRow {
    let access = block_on(issue_writer(&w.app, &w.principal(who), &w.slug_a, w.project_a)).unwrap();
    block_on(create_issue(&w.app, &access, d)).unwrap()
}

fn user(w: &World, who: Actor) -> UserId {
    w.principal(who).user_id
}

/// A work item that `who` made, which no route lets a guest do: inserted through the port, as `a_guest_reads_the_work_item_they_made` does.
fn seed_by(w: &World, who: Actor, n: u128, name: &str) -> WorkItemId {
    let scope =
        block_on(authorize(&w.app, &w.principal(who), &w.slug_a, w.project_a, Permission::ReadWorkItems)).unwrap();
    let mut tx = block_on(w.app.ports.db.begin(&scope)).unwrap();
    let alloc = block_on(tx.allocate(&scope)).unwrap();
    let item = WorkItem {
        id: WorkItemId(uuid(0x9000 + n)),
        workspace_id: w.ws_a,
        project_id: w.project_a,
        sequence_id: alloc.sequence_id,
        sort_order: alloc.sort_order,
        name: name.into(),
        description: None,
        external_source: None,
        external_id: None,
        created_by: user(w, who),
        created_at: w.app.ports.clock.now(),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    };
    block_on(tx.insert_work_item(&scope, &item)).unwrap();
    block_on(tx.commit()).unwrap();
    item.id
}

fn to_update(w: &World, who: Actor, id: WorkItemId) -> Result<(IssueAccess, WorkItemRow), IssueError> {
    block_on(issue_to_update(&w.app, &w.principal(who), &w.slug_a, w.project_a, id))
}

fn update(w: &World, who: Actor, id: WorkItemId, u: IssueUpdate) -> Result<(), IssueError> {
    let (access, before) = to_update(w, who, id)?;
    block_on(update_issue(&w.app, &access, before, u))
}

fn ask(asks: Vec<Ask>) -> IssueUpdate {
    IssueUpdate { asks, silent: false }
}

fn row(w: &World, id: WorkItemId) -> WorkItemRow {
    let access = block_on(issue_reader(&w.app, &w.principal(Actor::AdminA), &w.slug_a, w.project_a)).unwrap();
    block_on(get_issue(&w.app, &access, id)).unwrap()
}

fn name(s: &str) -> Ask {
    Ask::Name { sent: s.into(), name: s.trim().into() }
}

fn date(n: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 1).unwrap() + thrust_contracts::chrono::TimeDelta::days(n)
}

/// The consumer catching up: every event in the outbox, delivered once.
fn deliver(w: &World) {
    for row in block_on(w.app.ports.db.undispatched_outbox(&SystemScope::mint(), 1000)).unwrap() {
        block_on(process_event(&w.app, &row.envelope)).unwrap();
        block_on(w.app.ports.db.mark_outbox_dispatched(
            &SystemScope::mint(),
            row.envelope.event_id,
            w.app.ports.clock.now(),
        ))
        .unwrap();
    }
}

fn history(w: &World, id: WorkItemId) -> History {
    let access = block_on(issue_reader(&w.app, &w.principal(Actor::AdminA), &w.slug_a, w.project_a)).unwrap();
    block_on(issue_history(&w.app, &access, id, None)).unwrap()
}

/// The field of each row of a history, the creation's being none.
fn fields_of(h: &History) -> Vec<Option<String>> {
    h.rows.iter().map(|r| r.field.clone()).collect()
}

// --- who may ------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Of {
    /// A work item the caller made.
    Own,
    /// One a member made.
    Another,
    Missing,
}

/// Spec (the pin's `allow_permission` with `creator=True`): an admin or a member of the project changes any work item of it; a guest
/// only one they made; someone who is no member of the project (of its workspace or not) none. A work item that is not there is
/// `NotFound` to those who may change one and `Forbidden` to the rest, whom the guard stops before the work item is looked for.
#[rstest]
fn who_may_change_a_work_item(
    #[values(Actor::AdminA, Actor::MemberA, Actor::Dual, Actor::GuestA, Actor::OutsiderA, Actor::MemberB)] who: Actor,
    #[values(Of::Own, Of::Another, Of::Missing)] item: Of,
) {
    let w = seeded();
    let by_a_member = make(&w, Actor::MemberA, draft("a member's")).item.id;
    let id = match item {
        Of::Another => by_a_member,
        Of::Missing => WorkItemId(uuid(0xBAD)),
        // someone with no place in the project has no work item of their own to ask about
        Of::Own if matches!(who, Actor::OutsiderA | Actor::MemberB) => by_a_member,
        Of::Own => seed_by(&w, who, 1, "their own"),
    };
    let got = to_update(&w, who, id);
    let writes = matches!(who, Actor::AdminA | Actor::MemberA | Actor::Dual);
    match (writes, who, item) {
        (true, _, Of::Missing) => assert!(matches!(got, Err(IssueError::NotFound)), "{who:?} {item:?}: {got:?}"),
        (true, _, _) => assert_eq!(got.unwrap().1.item.id, id, "{who:?} {item:?}"),
        (false, Actor::GuestA, Of::Own) => assert_eq!(got.unwrap().1.item.id, id),
        (false, _, _) => assert!(matches!(got, Err(IssueError::Forbidden)), "{who:?} {item:?}: {got:?}"),
    }
}

/// Spec: the work item is looked for in the project the request names: another project's is not found, and a guard that fails first
/// is `Forbidden` whatever the work item is.
#[test]
fn a_work_item_of_another_project_is_not_found_and_a_stranger_is_forbidden_first() {
    let w = seeded();
    let id = make(&w, Actor::MemberA, draft("n")).item.id;
    // the dual member is a member of project A and a guest of project B: B is not where the work item is
    let in_b = block_on(issue_to_update(&w.app, &w.principal(Actor::MemberB), &w.slug_b, w.project_b, id));
    assert!(matches!(in_b, Err(IssueError::NotFound)), "{in_b:?}");
    let stranger = block_on(issue_to_update(&w.app, &w.principal(Actor::MemberB), &w.slug_a, w.project_a, id));
    assert!(matches!(stranger, Err(IssueError::Forbidden)), "{stranger:?}");
}

// --- what a change does ----------------------------------------------------------------------------------------------------------

/// What a change leaves of a work item, kept by the model of the test beside the real one.
#[derive(Debug, Clone, PartialEq)]
struct Model {
    name: String,
    description: String,
    priority: Priority,
    state: u128,
    completed: bool,
    start: Option<NaiveDate>,
    target: Option<NaiveDate>,
    assignees: BTreeSet<UserId>,
    labels: BTreeSet<Uuid>,
}

/// One thing a generated request names. The numbers pick the value.
#[derive(Debug, Clone)]
enum Spec {
    Name(u8),
    Description(u8),
    Priority(u8),
    State(Option<u8>),
    Start(Option<u8>),
    Target(Option<u8>),
    Assignees(u8),
    Labels(u8),
}

const STATES: [u128; 3] = [STATE_BACKLOG, STATE_TODO, STATE_DONE];

impl Spec {
    fn ask(&self, w: &World) -> Ask {
        match self {
            // a name is sent padded when its number is even
            Spec::Name(n) => Ask::Name {
                sent: if n % 2 == 0 { format!("  name {n}  ") } else { format!("name {n}") },
                name: format!("name {n}"),
            },
            Spec::Description(n) => Ask::Description(format!("<p>d{n}</p>")),
            Spec::Priority(n) => Ask::Priority(Priority::ALL[usize::from(*n) % 5]),
            Spec::State(s) => Ask::State(s.map(|s| uuid(STATES[usize::from(s) % 3]))),
            Spec::Start(d) => {
                Ask::Start { sent: d.map(|d| date(i64::from(d)).to_string()), date: d.map(|d| date(i64::from(d))) }
            }
            Spec::Target(d) => {
                Ask::Target { sent: d.map(|d| date(i64::from(d)).to_string()), date: d.map(|d| date(i64::from(d))) }
            }
            // a bit of the number for each of the six people, so guests, strangers and the other workspace's member are named too
            Spec::Assignees(bits) => Ask::Assignees(
                ACTORS.iter().enumerate().filter(|(i, _)| bits >> i & 1 == 1).map(|(_, a)| user(w, *a)).collect(),
            ),
            Spec::Labels(bits) => Ask::Labels(
                [0x7001u128, 0x7002]
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| bits >> i & 1 == 1)
                    .map(|(_, l)| uuid(*l))
                    .collect(),
            ),
        }
    }

    fn kind(&self) -> usize {
        match self {
            Spec::Name(_) => 0,
            Spec::Description(_) => 1,
            Spec::Priority(_) => 2,
            Spec::State(_) => 3,
            Spec::Start(_) => 4,
            Spec::Target(_) => 5,
            Spec::Assignees(_) => 6,
            Spec::Labels(_) => 7,
        }
    }
}

fn spec() -> impl Strategy<Value = Spec> {
    prop_oneof![
        (0..6u8).prop_map(Spec::Name),
        (0..6u8).prop_map(Spec::Description),
        (0..5u8).prop_map(Spec::Priority),
        prop::option::of(0..3u8).prop_map(Spec::State),
        prop::option::of(0..4u8).prop_map(Spec::Start),
        prop::option::of(0..4u8).prop_map(Spec::Target),
        (0..64u8).prop_map(Spec::Assignees),
        (0..4u8).prop_map(Spec::Labels),
    ]
}

/// A request: up to six things, each kind at most once (a JSON object names a field once), in any order.
fn request() -> impl Strategy<Value = Vec<Spec>> {
    prop::collection::vec(spec(), 0..7).prop_map(|specs| {
        let mut seen = BTreeSet::new();
        specs.into_iter().filter(|s| seen.insert(s.kind())).collect()
    })
}

/// Spec, written out apart from the code: what the work item is after a request. The state a request names, or the project's
/// default for null; completed when the state changes to the done one, and as it was when the state does not change; the people the
/// request names who are members of the project (not the guest, the stranger or the other workspace's member), once each; the
/// start not after the target, or the request is refused and nothing changes.
fn expected(before: &Model, w: &World, specs: &[Spec]) -> Result<Model, ()> {
    let mut m = before.clone();
    for s in specs {
        match (s, s.ask(w)) {
            (Spec::Name(_), Ask::Name { name, .. }) => m.name = name,
            (Spec::Description(_), Ask::Description(h)) => m.description = h,
            (Spec::Priority(_), Ask::Priority(p)) => m.priority = p,
            (Spec::State(to), _) => m.state = to.map_or(STATE_BACKLOG, |t| STATES[usize::from(t) % 3]),
            (Spec::Start(d), _) => m.start = d.map(|d| date(i64::from(d))),
            (Spec::Target(d), _) => m.target = d.map(|d| date(i64::from(d))),
            (Spec::Assignees(bits), _) => {
                m.assignees = ACTORS
                    .iter()
                    .enumerate()
                    .filter(|(i, a)| bits >> i & 1 == 1 && matches!(a, Actor::AdminA | Actor::MemberA | Actor::Dual))
                    .map(|(_, a)| user(w, *a))
                    .collect()
            }
            (Spec::Labels(bits), _) => {
                m.labels = [0x7001u128, 0x7002]
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| bits >> i & 1 == 1)
                    .map(|(_, l)| uuid(*l))
                    .collect()
            }
            _ => unreachable!(),
        }
    }
    if matches!((m.start, m.target), (Some(a), Some(b)) if a > b) {
        return Err(());
    }
    let moved = specs.iter().any(|s| matches!(s, Spec::State(_))) && m.state != before.state;
    m.completed = if moved { m.state == STATE_DONE } else { before.completed };
    Ok(m)
}

fn model_of(r: &WorkItemRow) -> Model {
    Model {
        name: r.item.name.clone(),
        description: r.attrs.description_html.clone(),
        priority: r.attrs.priority,
        state: r.attrs.state_id.map(|s| s.as_u128()).unwrap(),
        completed: r.attrs.completed_at.is_some(),
        start: r.attrs.start_date,
        target: r.attrs.target_date,
        assignees: r.attrs.assignee_ids.iter().copied().collect(),
        labels: r.attrs.label_ids.iter().copied().collect(),
    }
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// Spec: a request changes what it names and nothing else; `updated_at` and `updated_by` always move, and `completed_at` is set
    /// to the time of the change exactly when the state moves to the done one, cleared when it moves to another, and left alone when
    /// the state stays; a request that would leave the start after the target changes nothing at all and writes no event.
    #[test]
    fn prop_a_change_applies_what_it_names_and_leaves_the_rest(requests in prop::collection::vec((request(), 0..3usize), 1..6)) {
        let w = ticking_world();
        let created = make(&w, Actor::MemberA, draft("first"));
        let id = created.item.id;
        let mut model = model_of(&created);
        let doers = [Actor::AdminA, Actor::MemberA, Actor::Dual];
        for (specs, doer) in requests {
            let who = doers[doer];
            let before = row(&w, id);
            let events = block_on(outbox_depth(&w.app)).unwrap();
            let asks = specs.iter().map(|s| s.ask(&w)).collect();
            let got = update(&w, who, id, ask(asks));
            let after = row(&w, id);
            match expected(&model, &w, &specs) {
                Err(()) => {
                    prop_assert!(matches!(got, Err(IssueError::StartAfterTarget)), "{:?}", got);
                    prop_assert_eq!(&after, &before, "a refused change changes nothing");
                    prop_assert_eq!(block_on(outbox_depth(&w.app)).unwrap(), events, "and writes no event");
                }
                Ok(want) => {
                    prop_assert!(got.is_ok(), "{:?}", got);
                    prop_assert_eq!(model_of(&after), want.clone());
                    prop_assert_eq!(after.attrs.updated_by, Some(user(&w, who)));
                    prop_assert!(after.attrs.updated_at > before.attrs.updated_at);
                    prop_assert_eq!(after.item.id, id);
                    // the time the work item was completed at: the time of this change when it entered the done state, else as it was
                    let moved = specs.iter().any(|s| matches!(s, Spec::State(_))) && want.state != model.state;
                    if moved {
                        prop_assert_eq!(after.attrs.completed_at, want.completed.then_some(after.attrs.updated_at));
                    } else {
                        prop_assert_eq!(after.attrs.completed_at, before.attrs.completed_at);
                    }
                    // what the work item keeps that no request can change
                    prop_assert_eq!((after.item.sequence_id, after.item.sort_order, after.item.created_by), (before.item.sequence_id, before.item.sort_order, before.item.created_by));
                    prop_assert_eq!(block_on(outbox_depth(&w.app)).unwrap(), events + 1, "one event for a change");
                    model = want;
                }
            }
        }
    }
}

/// Spec (`Issue.save`): a work item with no state (one the slice made) is given the project's default when it is changed, whatever
/// the request names; the state group's `completed_at` follows.
#[test]
fn a_work_item_with_no_state_gets_the_default_when_it_is_changed() {
    let w = ticking_world();
    let id = seed_by(&w, Actor::MemberA, 7, "slice-made");
    assert_eq!(row(&w, id).attrs.state_id, None);
    update(&w, Actor::MemberA, id, ask(vec![Ask::Priority(Priority::High)])).unwrap();
    let after = row(&w, id);
    assert_eq!(
        (after.attrs.state_id, after.attrs.priority, after.attrs.completed_at),
        (Some(uuid(STATE_BACKLOG)), Priority::High, None)
    );
}

/// Spec: what a change names must be there, as for a creation: a state that is none of the project's, an assignee who is no user, a
/// label that is none of its own, a parent that is none of its work items; all found together, in the order of the request's fields
/// (state, parent, assignees, labels), and nothing written. A parent that is one of its work items is not supported yet.
#[test]
fn what_a_change_names_must_be_there_and_every_problem_is_found() {
    let w = seeded();
    let id = make(&w, Actor::MemberA, draft("n")).item.id;
    let other = make(&w, Actor::MemberA, draft("other")).item.id;
    let nobody = UserId(uuid(0xDEAD));
    let events = block_on(outbox_depth(&w.app)).unwrap();
    let asks = vec![
        Ask::Labels(vec![uuid(0x7003), uuid(0x7001), uuid(0xDEAD)]),
        Ask::Assignees(vec![user(&w, Actor::MemberA), nobody, nobody]),
        Ask::Parent(uuid(0xBAD)),
        Ask::State(Some(uuid(STATE_OF_B))),
        Ask::Priority(Priority::Urgent),
    ];
    let got = update(&w, Actor::MemberA, id, ask(asks));
    let Err(IssueError::Fields(problems)) = got else { panic!("not refused as fields: {got:?}") };
    assert_eq!(
        problems,
        vec![
            FieldProblem::State,
            FieldProblem::Parent,
            FieldProblem::Assignee(1),
            FieldProblem::Assignee(2),
            FieldProblem::Label(0),
            FieldProblem::Label(2)
        ]
    );
    assert_eq!(row(&w, id).attrs.priority, Priority::None, "a refused change writes nothing");
    assert_eq!(block_on(outbox_depth(&w.app)).unwrap(), events);
    let got = update(&w, Actor::MemberA, id, ask(vec![Ask::Parent(other.0)]));
    assert!(matches!(&got, Err(IssueError::Fields(p)) if *p == vec![FieldProblem::ParentNotSupported]), "{got:?}");
}

/// Spec (deviation D-029): the start must not be after the target, counting the date the work item has when the request names one
/// only; the pin accepts that, and stores the inverted pair.
#[rstest]
fn the_dates_must_stay_in_order_counting_the_ones_the_work_item_has(
    #[values(None, Some(1), Some(3))] had_start: Option<i64>,
    #[values(None, Some(1), Some(3))] had_target: Option<i64>,
    #[values(None, Some(0), Some(2), Some(4))] ask_start: Option<i64>,
    #[values(None, Some(0), Some(2), Some(4))] ask_target: Option<i64>,
) {
    // a work item cannot be made with an inverted pair, so only the ordered ones have a case
    if matches!((had_start, had_target), (Some(a), Some(b)) if a > b) {
        return;
    }
    let w = seeded();
    let id = make(
        &w,
        Actor::MemberA,
        IssueDraft { start_date: had_start.map(date), target_date: had_target.map(date), ..draft("n") },
    )
    .item
    .id;
    let mut asks = vec![];
    if let Some(d) = ask_start {
        asks.push(Ask::Start { sent: Some(date(d).to_string()), date: Some(date(d)) });
    }
    if let Some(d) = ask_target {
        asks.push(Ask::Target { sent: Some(date(d).to_string()), date: Some(date(d)) });
    }
    let (start, target) = (ask_start.or(had_start), ask_target.or(had_target));
    let got = update(&w, Actor::MemberA, id, ask(asks));
    assert_eq!(
        matches!(got, Err(IssueError::StartAfterTarget)),
        matches!((start, target), (Some(a), Some(b)) if a > b),
        "{had_start:?} {had_target:?} {ask_start:?} {ask_target:?}: {got:?}"
    );
}

// --- what a change records ------------------------------------------------------------------------------------------------------

fn names() -> Names {
    Names::new(
        [
            (uuid(STATE_BACKLOG), "Backlog".to_string()),
            (uuid(STATE_TODO), "Todo".to_string()),
            (uuid(STATE_DONE), "Done".to_string()),
        ],
        [(uuid(0x7001), "l1".to_string()), (uuid(0x7002), "l2".to_string())],
        (1..=6u128).map(|n| (UserId(uuid(n)), format!("person {n}"))),
    )
}

fn user_n(n: u128) -> UserId {
    UserId(uuid(n))
}

fn some_row(spec: &Model) -> WorkItemRow {
    let at = DateTime::from_timestamp(1_760_000_000, 0).unwrap();
    WorkItemRow {
        item: WorkItem {
            id: WorkItemId(uuid(0x9999)),
            workspace_id: WorkspaceId(uuid(0xA0)),
            project_id: ProjectId(uuid(0xA1)),
            sequence_id: 1,
            sort_order: 65_535,
            name: spec.name.clone(),
            description: None,
            external_source: None,
            external_id: None,
            created_by: user_n(1),
            created_at: at,
            archived_at: None,
            is_draft: false,
            deleted_at: None,
        },
        attrs: WorkItemAttrs {
            state_id: Some(uuid(spec.state)),
            priority: spec.priority,
            start_date: spec.start,
            target_date: spec.target,
            completed_at: None,
            updated_at: at,
            updated_by: None,
            description_html: spec.description.clone(),
            assignee_ids: spec.assignees.iter().copied().collect(),
            label_ids: spec.labels.iter().copied().collect(),
        },
        state_group: None,
    }
}

fn model() -> impl Strategy<Value = Model> {
    (0..6u8, 0..6u8, 0..5u8, 0..3u8, prop::option::of(0..4i64), prop::option::of(0..4i64), 0..64u8, 0..4u8).prop_map(
        |(n, d, p, s, start, target, people, labels)| Model {
            name: format!("name {n}"),
            description: format!("<p>d{d}</p>"),
            priority: Priority::ALL[usize::from(p)],
            state: STATES[usize::from(s)],
            completed: false,
            start: start.map(date),
            target: target.map(date),
            assignees: (1..=6u128).filter(|i| people >> (i - 1) & 1 == 1).map(user_n).collect(),
            labels: [0x7001u128, 0x7002]
                .iter()
                .enumerate()
                .filter(|(i, _)| labels >> i & 1 == 1)
                .map(|(_, l)| uuid(*l))
                .collect(),
        },
    )
}

/// The asks that say what a model already is, in the order the fields are listed.
fn asks_of(m: &Model) -> Vec<Ask> {
    vec![
        Ask::Name { sent: m.name.clone(), name: m.name.clone() },
        Ask::Description(m.description.clone()),
        Ask::Priority(m.priority),
        Ask::State(Some(uuid(m.state))),
        Ask::Start { sent: m.start.map(|d| d.to_string()), date: m.start },
        Ask::Target { sent: m.target.map(|d| d.to_string()), date: m.target },
        Ask::Assignees(m.assignees.iter().copied().collect()),
        Ask::Labels(m.labels.iter().copied().collect()),
    ]
}

proptest! {
    #![proptest_config(proptest_config(96))]

    /// Spec: asking for what the work item has is no change, so no row, whatever it has and in whatever order the fields come.
    #[test]
    fn prop_asking_for_what_is_there_records_nothing(m in model(), shuffle in any::<prop::sample::Index>()) {
        let mut asks = asks_of(&m);
        let n = asks.len();
        asks.rotate_left(shuffle.index(n));
        prop_assert_eq!(update_history(&some_row(&m), &asks, &names()), vec![]);
    }

    /// Spec (`issue_activities_task`): a row for each thing the request names that differs from what the work item has, in the order
    /// the request names them; a name, a description, a priority, a state and a date each one row, the people and the labels one row
    /// for each one added and each one dropped, the added first, each in the order of their ids. The row says what it was and what it
    /// is, and for a state, a person or a label which one.
    #[test]
    fn prop_a_row_for_each_difference_in_the_order_asked(before in model(), after in model(), order in Just((0..8usize).collect::<Vec<_>>()).prop_shuffle()) {
        let all = asks_of(&after);
        let asks: Vec<Ask> = order.iter().map(|i| all[*i].clone()).collect();
        let got = update_history(&some_row(&before), &asks, &names());

        let mut want: Vec<(String, Option<String>, Option<String>, String)> = Vec::new();
        let day = |d: Option<NaiveDate>| d.map(|d| d.to_string()).unwrap_or_default();
        for i in &order {
            match i {
                0 if before.name != after.name => want.push(("name".into(), Some(before.name.clone()), Some(after.name.clone()), "updated the name to".into())),
                1 if before.description != after.description => want.push(("description".into(), Some(before.description.clone()), Some(after.description.clone()), "updated the description to".into())),
                2 if before.priority != after.priority => want.push(("priority".into(), Some(before.priority.as_str().into()), Some(after.priority.as_str().into()), "updated the priority to".into())),
                3 if before.state != after.state => {
                    let named = |s: u128| ["Backlog", "Todo", "Done"][STATES.iter().position(|x| *x == s).unwrap()].to_string();
                    want.push(("state".into(), Some(named(before.state)), Some(named(after.state)), "updated the state to".into()))
                }
                4 if before.start != after.start => want.push(("start_date".into(), Some(day(before.start)), Some(day(after.start)), "updated the start date to ".into())),
                5 if before.target != after.target => want.push(("target_date".into(), Some(day(before.target)), Some(day(after.target)), "updated the target date to".into())),
                6 => {
                    for u in after.assignees.difference(&before.assignees) {
                        want.push(("assignees".into(), Some(String::new()), Some(format!("person {}", u.0.as_u128())), "added assignee ".into()));
                    }
                    for u in before.assignees.difference(&after.assignees) {
                        want.push(("assignees".into(), Some(format!("person {}", u.0.as_u128())), Some(String::new()), "removed assignee ".into()));
                    }
                }
                7 => {
                    for l in after.labels.difference(&before.labels) {
                        want.push(("labels".into(), Some(String::new()), Some(format!("l{}", l.as_u128() - 0x7000)), "added label ".into()));
                    }
                    for l in before.labels.difference(&after.labels) {
                        want.push(("labels".into(), Some(format!("l{}", l.as_u128() - 0x7000)), Some(String::new()), "removed label ".into()));
                    }
                }
                _ => {}
            }
        }
        let got_shown: Vec<_> = got.iter().map(|c| (c.field.clone().unwrap(), c.old_value.clone(), c.new_value.clone(), c.comment.clone())).collect();
        prop_assert_eq!(got_shown, want);
        prop_assert!(got.iter().all(|c| c.verb == "updated"));
        // which state, person or label a row is about is in its identifiers: the new one for an addition, the old one for a drop
        for c in &got {
            match (c.field.as_deref(), c.comment.as_str()) {
                (Some("assignees" | "labels"), "added assignee " | "added label ") => prop_assert!(c.new_identifier.is_some() && c.old_identifier.is_none()),
                (Some("assignees" | "labels"), _) => prop_assert!(c.old_identifier.is_some() && c.new_identifier.is_none()),
                (Some("state"), _) => prop_assert_eq!((c.old_identifier, c.new_identifier), (Some(uuid(before.state)), Some(uuid(after.state)))),
                _ => prop_assert!(c.old_identifier.is_none() && c.new_identifier.is_none()),
            }
        }
        // and it travels as JSON in the event
        let json = serde_json::to_value(&got).unwrap();
        prop_assert_eq!(serde_json::from_value::<Vec<HistoryChange>>(json).unwrap(), got);
    }
}

/// Spec (`track_name`, `track_start_date`): the pin compares what was *asked*, as text, with what the work item has: a name sent
/// with spaces around it is a change though it is the name the work item has, and a day sent in another spelling than the one the
/// work item shows it in is a change; null for a date is the empty text, which is a change only from a date.
#[test]
fn what_was_asked_is_compared_as_the_text_it_came_in() {
    let before = some_row(&Model {
        name: "n".into(),
        description: "<p></p>".into(),
        priority: Priority::None,
        state: STATE_BACKLOG,
        completed: false,
        start: Some(date(0)),
        target: None,
        assignees: BTreeSet::new(),
        labels: BTreeSet::new(),
    });
    let asks = vec![
        Ask::Name { sent: "  n  ".into(), name: "n".into() },
        Ask::Start { sent: Some("20261101".into()), date: Some(date(0)) },
        Ask::Target { sent: None, date: None },
    ];
    let got = update_history(&before, &asks, &names());
    let fields: Vec<(&str, &str, &str)> = got
        .iter()
        .map(|c| (c.field.as_deref().unwrap(), c.old_value.as_deref().unwrap(), c.new_value.as_deref().unwrap()))
        .collect();
    assert_eq!(fields, vec![("name", "n", "  n  "), ("start_date", "2026-11-01", "20261101")]);
}

/// Spec (deviation D-029): a value in a history row is cut at 16 KiB, so the event that carries it stays under the queue's message
/// limit, on a character boundary.
#[rstest]
fn a_long_value_is_cut_on_a_character_boundary(#[values("a", "é", "€", "𝄞")] unit: &str) {
    let long = unit.repeat(40_000);
    let before = some_row(&Model {
        name: "n".into(),
        description: "<p></p>".into(),
        priority: Priority::None,
        state: STATE_BACKLOG,
        completed: false,
        start: None,
        target: None,
        assignees: BTreeSet::new(),
        labels: BTreeSet::new(),
    });
    let got = update_history(&before, &[Ask::Description(long.clone())], &names());
    let kept = got[0].new_value.as_deref().unwrap();
    assert!(kept.len() <= 16 * 1024 && kept.len() > 16 * 1024 - 4, "{}", kept.len());
    assert!(long.starts_with(kept), "what is kept is the start of the value");
}

// --- the history, written by the consumer -----------------------------------------------------------------------------------------

fn profile(w: &World, who: Actor, display: &str) {
    let now = w.app.ports.clock.now();
    let p = UserProfile {
        id: uuid(0xF000 + user(w, who).0.as_u128()),
        user_id: user(w, who),
        username: format!("u{}", user(w, who).0.as_u128()),
        first_name: "F".into(),
        last_name: "L".into(),
        display_name: display.into(),
        avatar: String::new(),
        cover_image: None,
        timezone: "UTC".into(),
        date_joined: now,
        last_login_at: None,
        is_onboarded: true,
        is_tour_completed: true,
        last_workspace_id: None,
        onboarding_step: serde_json::json!({}),
        preferences: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    };
    assert!(block_on(w.app.ports.db.insert_profile(&p)).unwrap());
}

/// Spec (`create_issue_activity`): a creation's history says it was made, by whom, and then that each person the request named was
/// assigned, once each, in the order of their ids, whether or not the work item keeps them (a guest is named and not kept); the
/// default assignee is not named by the request and is not recorded. Their names are those they are called by now.
#[test]
fn a_creation_is_in_the_history_with_the_people_it_named() {
    let w = seeded();
    profile(&w, Actor::AdminA, "Ada");
    profile(&w, Actor::GuestA, "Gus");
    profile(&w, Actor::MemberA, "Mia");
    let (admin, guest) = (user(&w, Actor::AdminA), user(&w, Actor::GuestA));
    let created = make(&w, Actor::MemberA, IssueDraft { assignee_ids: vec![guest, admin, guest], ..draft("made") });
    assert_eq!(created.attrs.assignee_ids, vec![admin], "the guest is not kept");
    deliver(&w);
    let h = history(&w, created.item.id);
    let rows: Vec<_> = h
        .rows
        .iter()
        .map(|r| (r.verb.as_str(), r.field.as_deref(), r.new_value.as_deref(), r.comment.as_str(), r.new_identifier))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("created", None, None, "created the issue", None),
            ("updated", Some("assignees"), Some("Ada"), "added assignee ", Some(admin.0)),
            ("updated", Some("assignees"), Some("Gus"), "added assignee ", Some(guest.0)),
        ]
    );
    assert!(h.rows.iter().all(|r| r.actor_id == Some(user(&w, Actor::MemberA))
        && r.epoch == 1_760_000_000
        && r.old_identifier.is_none()));
    assert_eq!(h.rows.iter().map(|r| r.ordinal).collect::<Vec<_>>(), vec![0, 1, 2]);
    assert!(h.item.is_some() && h.project.is_some() && h.workspace.is_some());
    assert_eq!(
        h.people.keys().copied().collect::<Vec<_>>(),
        vec![user(&w, Actor::MemberA)],
        "the people who made the changes"
    );
}

/// Spec: a change is one `work_item.updated` event with the history it records and the fields it named; one that asks for nothing
/// different still writes the event (the work item was changed: `updated_at`), with an empty history; and a description written by a
/// migration (`silent`) writes no event at all.
#[rstest]
fn a_change_writes_its_event_and_a_silent_one_does_not(
    #[values(false, true)] silent: bool,
    #[values(false, true)] differs: bool,
) {
    let w = seeded();
    let id = make(&w, Actor::MemberA, draft("n")).item.id;
    let before = block_on(w.app.ports.db.undispatched_outbox(&SystemScope::mint(), 100)).unwrap().len();
    let new_name = if differs { "m" } else { "n" };
    update(&w, Actor::MemberA, id, IssueUpdate { asks: vec![name(new_name), Ask::Priority(Priority::None)], silent })
        .unwrap();
    let rows = block_on(w.app.ports.db.undispatched_outbox(&SystemScope::mint(), 100)).unwrap();
    assert_eq!(rows.len(), before + usize::from(!silent));
    if silent {
        return;
    }
    let env = &rows.last().unwrap().envelope;
    assert_eq!(
        (env.event_type.as_str(), env.entity_type.as_str(), env.entity_id, env.actor_id),
        ("work_item.updated", "work_item", id.0, Some(user(&w, Actor::MemberA)))
    );
    assert_eq!(env.payload["changed_fields"], serde_json::json!(["name", "priority"]));
    assert_eq!(env.payload["project_id"], serde_json::json!(w.project_a));
    let history: Vec<HistoryChange> = serde_json::from_value(env.payload["history"].clone()).unwrap();
    assert_eq!(history.len(), usize::from(differs));
}

/// Spec (idempotent consumer): delivering an event again writes no row twice, however many times and whatever came between.
#[test]
fn an_event_delivered_again_writes_no_row_twice() {
    let w = seeded();
    let id = make(&w, Actor::MemberA, draft("n")).item.id;
    update(&w, Actor::MemberA, id, ask(vec![name("m"), Ask::Priority(Priority::Low)])).unwrap();
    let events: Vec<_> = block_on(w.app.ports.db.undispatched_outbox(&SystemScope::mint(), 100))
        .unwrap()
        .into_iter()
        .map(|r| r.envelope)
        .collect();
    assert_eq!(events.len(), 2);
    for env in events.iter().chain(events.iter().rev()).chain(events.iter()) {
        block_on(process_event(&w.app, env)).unwrap();
    }
    let h = history(&w, id);
    assert_eq!(fields_of(&h), vec![None, Some("name".into()), Some("priority".into())]);
}

/// What the consumer does with a run of changes to descriptions and names, written out apart from the code: a description change by
/// the person who made the newest row, when that row is a description change, moves that row to the time of the change and adds
/// none; any other change is a row of its own.
fn expected_rows(changes: &[(usize, bool)]) -> Vec<(bool, usize)> {
    // (is_description, doer) in the order of their time; the creation row is made by doer 0 and is no description
    let mut rows: Vec<(bool, usize)> = vec![(false, 0)];
    for (doer, is_description) in changes {
        if *is_description && rows.last() == Some(&(true, *doer)) {
            continue;
        }
        rows.push((*is_description, *doer));
    }
    rows
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// Spec (`track_description`): typing in the editor is one change: a description change by the same person right after their last
    /// one (nothing of anyone's in between) is not a row of its own.
    #[test]
    fn prop_a_description_change_joins_the_one_before_it_by_the_same_person(changes in prop::collection::vec((0..2usize, any::<bool>()), 1..10)) {
        let w = ticking_world();
        let id = make(&w, Actor::AdminA, draft("start")).item.id;
        let doers = [Actor::AdminA, Actor::MemberA];
        deliver(&w);
        for (i, (doer, is_description)) in changes.iter().enumerate() {
            // each is a change: a name or a description nobody had before
            let a = if *is_description { Ask::Description(format!("<p>{i}</p>")) } else { name(&format!("n{i}")) };
            update(&w, doers[*doer], id, ask(vec![a])).unwrap();
            deliver(&w);
        }
        let h = history(&w, id);
        let got: Vec<(bool, usize)> = h
            .rows
            .iter()
            .map(|r| (r.field.as_deref() == Some("description"), doers.iter().position(|a| Some(user(&w, *a)) == r.actor_id).unwrap()))
            .collect();
        // the creation row is by the admin, the first doer, and is not a description
        prop_assert_eq!(got, expected_rows(&changes));
        // the rows are oldest first by time: a row that was joined moved to the end
        let times: Vec<_> = h.rows.iter().map(|r| r.created_at).collect();
        let mut sorted = times.clone();
        sorted.sort();
        prop_assert_eq!(times, sorted);
    }
}

/// Spec: a description change is joined only to a description change of the same work item: another work item's newest row does not
/// count.
#[test]
fn a_description_change_does_not_join_another_work_items_row() {
    let w = ticking_world();
    let a = make(&w, Actor::AdminA, draft("a")).item.id;
    let b = make(&w, Actor::AdminA, draft("b")).item.id;
    update(&w, Actor::AdminA, a, ask(vec![Ask::Description("<p>1</p>".into())])).unwrap();
    update(&w, Actor::AdminA, b, ask(vec![Ask::Description("<p>2</p>".into())])).unwrap();
    update(&w, Actor::AdminA, a, ask(vec![Ask::Description("<p>3</p>".into())])).unwrap();
    deliver(&w);
    let fields = |id| history(&w, id).rows.iter().map(|r| r.field.clone()).collect::<Vec<_>>();
    assert_eq!(fields(a), vec![None, Some("description".to_string())], "the third is joined to the first");
    assert_eq!(fields(b), vec![None, Some("description".to_string())]);
}

/// Spec: the history is read oldest first, only the rows made after a time when one is given (strictly), only of the work item
/// asked for, and of one that is not there it is empty, not an error.
#[test]
fn the_history_is_read_after_a_time_and_only_of_its_work_item() {
    let w = ticking_world();
    let a = make(&w, Actor::AdminA, draft("a")).item.id;
    let b = make(&w, Actor::AdminA, draft("b")).item.id;
    update(&w, Actor::AdminA, a, ask(vec![name("a2")])).unwrap();
    update(&w, Actor::AdminA, b, ask(vec![name("b2")])).unwrap();
    update(&w, Actor::AdminA, a, ask(vec![Ask::Priority(Priority::Low)])).unwrap();
    deliver(&w);
    let access = block_on(issue_reader(&w.app, &w.principal(Actor::GuestA), &w.slug_a, w.project_a)).unwrap();
    let read = |id: WorkItemId, after: Option<Timestamp>| block_on(issue_history(&w.app, &access, id, after)).unwrap();
    let all = read(a, None);
    assert_eq!(all.rows.len(), 3, "a guest reads the history of a work item they did not make");
    let times: Vec<_> = all.rows.iter().map(|r| r.created_at).collect();
    assert!(times.windows(2).all(|p| p[0] <= p[1]));
    assert_eq!(read(a, Some(times[0])).rows.len(), 2, "after is strict");
    assert_eq!(read(a, Some(times[2])).rows.len(), 0);
    assert_eq!(read(a, Some(times[1] - thrust_contracts::chrono::TimeDelta::nanoseconds(1))).rows.len(), 2);
    assert_eq!(read(b, None).rows.len(), 2);
    let none = read(WorkItemId(uuid(0xBAD)), None);
    assert!(none.rows.is_empty() && none.item.is_none());
    assert!(block_on(issue_there(&w.app, &access, a)).unwrap());
    assert!(!block_on(issue_there(&w.app, &access, WorkItemId(uuid(0xBAD)))).unwrap());
}
