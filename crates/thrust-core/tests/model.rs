//! Work items against a reference model: random sequences of create, delete and list, by every kind of actor, in
//! two tenants. Each step's result is compared with the model, which encodes the spec:
//!
//! * names are validated before anything else (1..=255 characters after trimming);
//! * a slug that names no workspace, a workspace the caller is not in and a project that is not in the named
//!   workspace are Not Found, a project the caller is not a member of is Forbidden;
//! * creating and deleting need Member, listing needs Guest, listing deleted rows needs Admin;
//! * sequence ids are per project, start at 1, increase by one per create and are never reused (ADR-0043 B1);
//! * a soft-deleted item disappears from the default list and stays in the with-deleted list;
//! * a list only ever returns rows of the requested project in the caller's workspace (R-S1).
//!
//! `expect` is the spec for one step and `apply` its effect on the model; the test runs them against the use-cases,
//! and `model_strategy_reaches_every_step_class` replays sampled sequences through them alone (the generator coverage
//! control, Rule 8).

mod common;

use std::collections::BTreeSet;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{TestCaseError, TestRunner};
use thrust_contracts::*;
use thrust_core::*;

#[derive(Debug, Clone)]
enum Op {
    Create { actor: Actor, slug: Slug, proj: Proj, valid_name: bool },
    /// `pick` chooses one of the project's items (live or deleted); with none, a random unknown id.
    Delete { actor: Actor, slug: Slug, proj: Proj, pick: usize },
    List { actor: Actor, slug: Slug, proj: Proj, with_deleted: bool },
}

fn actor() -> impl Strategy<Value = Actor> {
    prop::sample::select(ACTORS.to_vec())
}

fn proj() -> impl Strategy<Value = Proj> {
    prop_oneof![Just(Proj::A), Just(Proj::B)]
}

/// Mostly the project's own workspace, so work items accumulate; the rest probe the wrong and unknown slugs.
fn slug() -> impl Strategy<Value = Slug> {
    prop_oneof![8 => Just(Slug::Own), 1 => Just(Slug::Other), 1 => Just(Slug::Unknown)]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (actor(), slug(), proj(), prop::bool::weighted(0.9))
            .prop_map(|(actor, slug, proj, valid_name)| Op::Create { actor, slug, proj, valid_name }),
        2 => (actor(), slug(), proj(), 0..16usize)
            .prop_map(|(actor, slug, proj, pick)| Op::Delete { actor, slug, proj, pick }),
        2 => (actor(), slug(), proj(), any::<bool>())
            .prop_map(|(actor, slug, proj, with_deleted)| Op::List { actor, slug, proj, with_deleted }),
    ]
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(op(), 1..48)
}

#[derive(Debug, Clone, Copy)]
struct ModelItem {
    id: WorkItemId,
    seq: i64,
    deleted: bool,
}

#[derive(Default)]
struct Model {
    items: [Vec<ModelItem>; 2],
    last_seq: [i64; 2],
}

fn slot(p: Proj) -> usize {
    match p {
        Proj::A => 0,
        Proj::B => 1,
    }
}

/// The spec's access rule for `actor` on `proj` through `slug`, needing at least `min`: the slug must name the
/// project's own workspace, the actor must belong to it, and then the actor's role in the project decides.
fn access(actor: Actor, slug: Slug, proj: Proj, min: Role) -> Outcome {
    // (belongs to workspace A, belongs to workspace B), and the role in (project A, project B)
    let (in_ws, roles): ([bool; 2], [Option<Role>; 2]) = match actor {
        Actor::AdminA => ([true, false], [Some(Role::Admin), None]),
        Actor::MemberA => ([true, false], [Some(Role::Member), None]),
        Actor::GuestA => ([true, false], [Some(Role::Guest), None]),
        Actor::OutsiderA => ([true, false], [None, None]),
        Actor::MemberB => ([false, true], [None, Some(Role::Member)]),
        Actor::Dual => ([true, true], [Some(Role::Member), Some(Role::Guest)]),
    };
    let s = slot(proj);
    if slug != Slug::Own || !in_ws[s] {
        return Outcome::NotFound;
    }
    match roles[s] {
        None => Outcome::Forbidden,
        Some(role) if role >= min => Outcome::Ok,
        Some(_) => Outcome::Forbidden,
    }
}

/// What kind of step the spec predicted. The generator coverage control requires every one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Created,
    /// A create after a delete in the same project: where a reused sequence id would show.
    CreatedAfterDelete,
    InvalidName,
    /// The project's own slug, by someone who is not in that workspace.
    CrossTenant,
    /// A real workspace's slug, for a project that is in the other one.
    WrongSlug,
    /// A slug that names no workspace.
    UnknownSlug,
    NotAMember,
    RoleTooLow,
    Deleted,
    DeleteAlreadyDeleted,
    DeleteUnknown,
    Listed,
    /// A default list while the project has a deleted row, which it must hide.
    ListedHidingADeletedRow,
    ListedWithDeleted,
}

const ALL_CLASSES: [Class; 14] = [
    Class::Created,
    Class::CreatedAfterDelete,
    Class::InvalidName,
    Class::CrossTenant,
    Class::WrongSlug,
    Class::UnknownSlug,
    Class::NotAMember,
    Class::RoleTooLow,
    Class::Deleted,
    Class::DeleteAlreadyDeleted,
    Class::DeleteUnknown,
    Class::Listed,
    Class::ListedHidingADeletedRow,
    Class::ListedWithDeleted,
];

/// The spec for one step.
struct Expect {
    want: Outcome,
    class: Class,
    /// For a delete: the model item it targets, if the project has any.
    target: Option<usize>,
}

fn denied(actor: Actor, slug: Slug, want: &Outcome) -> Class {
    match (want, slug, actor) {
        (Outcome::NotFound, Slug::Unknown, _) => Class::UnknownSlug,
        (Outcome::NotFound, Slug::Other, _) => Class::WrongSlug,
        (Outcome::NotFound, Slug::Own, _) => Class::CrossTenant,
        (_, _, Actor::OutsiderA) => Class::NotAMember,
        _ => Class::RoleTooLow,
    }
}

fn expect(m: &Model, op: &Op) -> Expect {
    match *op {
        Op::Create { actor, slug, proj, valid_name } => {
            let s = slot(proj);
            let want = if valid_name { access(actor, slug, proj, Role::Member) } else { Outcome::Invalid };
            let class = match want {
                Outcome::Invalid => Class::InvalidName,
                Outcome::Ok if m.items[s].iter().any(|i| i.deleted) => Class::CreatedAfterDelete,
                Outcome::Ok => Class::Created,
                ref w => denied(actor, slug, w),
            };
            Expect { want, class, target: None }
        }
        Op::Delete { actor, slug, proj, pick } => {
            let s = slot(proj);
            let target = (!m.items[s].is_empty()).then(|| pick % m.items[s].len());
            let mut want = access(actor, slug, proj, Role::Member);
            let class = match (&want, target) {
                (Outcome::Ok, Some(i)) if !m.items[s][i].deleted => Class::Deleted,
                (Outcome::Ok, Some(_)) => Class::DeleteAlreadyDeleted,
                (Outcome::Ok, None) => Class::DeleteUnknown,
                (w, _) => denied(actor, slug, w),
            };
            if matches!(class, Class::DeleteAlreadyDeleted | Class::DeleteUnknown) {
                want = Outcome::NotFound;
            }
            Expect { want, class, target }
        }
        Op::List { actor, slug, proj, with_deleted } => {
            let min = if with_deleted { Role::Admin } else { Role::Guest };
            let want = access(actor, slug, proj, min);
            let class = match want {
                Outcome::Ok if with_deleted => Class::ListedWithDeleted,
                Outcome::Ok if m.items[slot(proj)].iter().any(|i| i.deleted) => Class::ListedHidingADeletedRow,
                Outcome::Ok => Class::Listed,
                ref w => denied(actor, slug, w),
            };
            Expect { want, class, target: None }
        }
    }
}

/// The step's effect on the model, once the use-case has done it: `created` is the new item's id and sequence id.
fn apply(m: &mut Model, op: &Op, e: &Expect, created: Option<(WorkItemId, i64)>) {
    if e.want != Outcome::Ok {
        return;
    }
    match *op {
        Op::Create { proj, .. } => {
            let (id, seq) = created.expect("a successful create has an item");
            let s = slot(proj);
            m.last_seq[s] = seq;
            m.items[s].push(ModelItem { id, seq, deleted: false });
        }
        Op::Delete { proj, .. } => {
            if let Some(i) = e.target {
                m.items[slot(proj)][i].deleted = true;
            }
        }
        Op::List { .. } => {}
    }
}

fn run(ops: &[Op]) -> Result<(), TestCaseError> {
    let w = world();
    let mut m = Model::default();
    for (step, op) in ops.iter().enumerate() {
        let e = expect(&m, op);
        let mut created = None;
        match *op {
            Op::Create { actor, slug, proj, valid_name } => {
                let name = if valid_name { format!("item {step}") } else { "   ".to_string() };
                let req = CreateWorkItemRequest { name, description: None, external_source: None, external_id: None };
                let got = block_on(create_work_item(&w.app, &w.principal(actor), &w.slug(proj, slug), w.project(proj), req));
                prop_assert_eq!(outcome(&got), e.want.clone(), "step {}: {:?}", step, op);
                if let Ok(item) = got {
                    let s = slot(proj);
                    prop_assert_eq!(item.sequence_id, m.last_seq[s] + 1, "step {}: sequence ids must not repeat", step);
                    prop_assert_eq!((item.workspace_id, item.project_id), (w.workspace_of(proj), w.project(proj)));
                    created = Some((item.id, item.sequence_id));
                }
            }
            Op::Delete { actor, slug, proj, pick } => {
                let s = slot(proj);
                let id = e.target.map_or(WorkItemId(Uuid::from_u128(0xBAD0 + pick as u128)), |i| m.items[s][i].id);
                let got = block_on(delete_work_item(&w.app, &w.principal(actor), &w.slug(proj, slug), w.project(proj), id));
                prop_assert_eq!(outcome(&got), e.want.clone(), "step {}: {:?}", step, op);
            }
            Op::List { actor, slug, proj, with_deleted } => {
                let vis = if with_deleted { Visibility::WithDeleted } else { Visibility::Visible };
                let got = block_on(list_work_items(&w.app, &w.principal(actor), &w.slug(proj, slug), w.project(proj), vis));
                prop_assert_eq!(outcome(&got), e.want.clone(), "step {}: {:?}", step, op);
                if let Ok(rows) = got {
                    for r in &rows {
                        prop_assert_eq!((r.workspace_id, r.project_id), (w.workspace_of(proj), w.project(proj)),
                            "step {}: a list returned a row outside its scope", step);
                    }
                    let seqs: Vec<i64> = rows.iter().map(|r| r.sequence_id).collect();
                    let want: Vec<i64> = m.items[slot(proj)].iter().filter(|i| with_deleted || !i.deleted).map(|i| i.seq).collect();
                    prop_assert_eq!(seqs, want, "step {}: {:?}", step, op);
                }
            }
        }
        apply(&mut m, op, &e, created);
    }
    Ok(())
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    #[test]
    fn model_work_items_match_the_reference_model(ops in ops()) {
        run(&ops)?;
    }
}

/// Rule 8 for the test above: a generator that never deletes an already deleted item, say, would pass on core code
/// that gets that wrong. Samples as many sequences as one run draws, with a fixed seed, replays them through `expect`
/// and `apply` alone and requires every step class at least once.
#[test]
fn model_strategy_reaches_every_step_class() {
    let mut runner = TestRunner::deterministic();
    let mut seen = BTreeSet::new();
    // the model test's own budget: what one run reaches
    for _ in 0..256 {
        let ops = ops().new_tree(&mut runner).unwrap().current();
        let mut m = Model::default();
        for (step, op) in ops.iter().enumerate() {
            let e = expect(&m, op);
            seen.insert(e.class);
            let seq = |m: &Model, p: Proj| m.last_seq[slot(p)] + 1;
            let created = match *op {
                Op::Create { proj, .. } => Some((WorkItemId(Uuid::from_u128(step as u128 + 1)), seq(&m, proj))),
                _ => None,
            };
            apply(&mut m, op, &e, created);
        }
    }
    let missing: Vec<_> = ALL_CLASSES.iter().filter(|c| !seen.contains(c)).collect();
    assert!(missing.is_empty(), "never generated: {missing:?}");
}
