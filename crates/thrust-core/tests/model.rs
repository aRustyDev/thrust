//! Work items against a reference model: random sequences of create, delete and list, by every kind of actor, in
//! two tenants. Each step's result is compared with the model, which encodes the spec:
//!
//! * names are validated before anything else (1..=255 characters after trimming);
//! * a project in another workspace is Not Found, a project the caller is not a member of is Forbidden;
//! * creating and deleting need Member, listing needs Guest, listing deleted rows needs Admin;
//! * sequence ids are per project, start at 1, increase by one per create and are never reused (ADR-0043 B1);
//! * a soft-deleted item disappears from the default list and stays in the with-deleted list;
//! * a list only ever returns rows of the requested project in the caller's workspace (R-S1).

mod common;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use thrust_contracts::*;
use thrust_core::*;

#[derive(Debug, Clone)]
enum Op {
    Create { actor: Actor, proj: Proj, valid_name: bool },
    /// `pick` chooses one of the project's items (live or deleted); with none, a random unknown id.
    Delete { actor: Actor, proj: Proj, pick: usize },
    List { actor: Actor, proj: Proj, with_deleted: bool },
}

fn actor() -> impl Strategy<Value = Actor> {
    prop::sample::select(ACTORS.to_vec())
}

fn proj() -> impl Strategy<Value = Proj> {
    prop_oneof![Just(Proj::A), Just(Proj::B)]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (actor(), proj(), prop::bool::weighted(0.9))
            .prop_map(|(actor, proj, valid_name)| Op::Create { actor, proj, valid_name }),
        2 => (actor(), proj(), 0..16usize).prop_map(|(actor, proj, pick)| Op::Delete { actor, proj, pick }),
        2 => (actor(), proj(), any::<bool>()).prop_map(|(actor, proj, with_deleted)| Op::List { actor, proj, with_deleted }),
    ]
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

/// The spec's access rule for `actor` on `proj`, needing `min` (None = no role needed beyond membership).
fn access(actor: Actor, proj: Proj, min: Role) -> Outcome {
    let role = match (actor, proj) {
        (Actor::MemberB, Proj::A) | (Actor::AdminA | Actor::MemberA | Actor::GuestA | Actor::OutsiderA, Proj::B) => {
            return Outcome::NotFound
        }
        (Actor::OutsiderA, Proj::A) => return Outcome::Forbidden,
        (Actor::AdminA, _) => Role::Admin,
        (Actor::MemberA | Actor::MemberB, _) => Role::Member,
        (Actor::GuestA, _) => Role::Guest,
    };
    if role >= min {
        Outcome::Ok
    } else {
        Outcome::Forbidden
    }
}

fn run(ops: &[Op]) -> Result<(), TestCaseError> {
    let w = world();
    let mut m = Model::default();
    for (step, op) in ops.iter().enumerate() {
        match *op {
            Op::Create { actor, proj, valid_name } => {
                let name = if valid_name { format!("item {step}") } else { "   ".to_string() };
                let req = CreateWorkItemRequest { name, description: None, external_source: None, external_id: None };
                let got = block_on(create_work_item(&w.app, &w.principal(actor), w.project(proj), req));
                let want = if valid_name { access(actor, proj, Role::Member) } else { Outcome::Invalid };
                prop_assert_eq!(outcome(&got), want.clone(), "step {}: {:?}", step, op);
                if let Ok(item) = got {
                    let s = slot(proj);
                    prop_assert_eq!(item.sequence_id, m.last_seq[s] + 1, "step {}: sequence ids must not repeat", step);
                    prop_assert_eq!((item.workspace_id, item.project_id), (w.workspace_of(proj), w.project(proj)));
                    m.last_seq[s] = item.sequence_id;
                    m.items[s].push(ModelItem { id: item.id, seq: item.sequence_id, deleted: false });
                }
            }
            Op::Delete { actor, proj, pick } => {
                let s = slot(proj);
                let target = if m.items[s].is_empty() {
                    None
                } else {
                    Some(pick % m.items[s].len())
                };
                let id = target.map_or(WorkItemId(Uuid::from_u128(0xBAD0 + pick as u128)), |i| m.items[s][i].id);
                let got = block_on(delete_work_item(&w.app, &w.principal(actor), w.project(proj), id));
                let mut want = access(actor, proj, Role::Member);
                if want == Outcome::Ok {
                    match target {
                        Some(i) if !m.items[s][i].deleted => m.items[s][i].deleted = true,
                        _ => want = Outcome::NotFound,
                    }
                }
                prop_assert_eq!(outcome(&got), want, "step {}: {:?}", step, op);
            }
            Op::List { actor, proj, with_deleted } => {
                let vis = if with_deleted { Visibility::WithDeleted } else { Visibility::Visible };
                let min = if with_deleted { Role::Admin } else { Role::Guest };
                let got = block_on(list_work_items(&w.app, &w.principal(actor), w.project(proj), vis));
                prop_assert_eq!(outcome(&got), access(actor, proj, min), "step {}: {:?}", step, op);
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
    }
    Ok(())
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    #[test]
    fn model_work_items_match_the_reference_model(ops in prop::collection::vec(op(), 1..48)) {
        run(&ops)?;
    }
}
