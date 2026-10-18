// TEMPLATE (rust-test-logic), adapted from rust-testing-plan.zip, 2026-10-18: copy to tests/model.rs; KvStore is an EXAMPLE type: substitute your own stateful type and crate (needs proptest-state-machine = "0.9" and tests/common/mod.rs).
//! Model-based tests: the real KvStore must behave exactly like std's BTreeMap.

use your_crate::store::KvStore;
use proptest::prelude::*;
use proptest_state_machine::{prop_state_machine, ReferenceStateMachine, StateMachineTest};
use std::collections::BTreeMap;

mod common;

#[derive(Clone, Debug)]
enum Op {
    Insert(String, i64),
    Get(String),
    Remove(String),
}

// A tiny key space makes operations collide on the same keys, which is where bugs live.
fn key() -> impl Strategy<Value = String> {
    "[a-d]"
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (key(), any::<i64>()).prop_map(|(k, v)| Op::Insert(k, v)),
        key().prop_map(Op::Get),
        key().prop_map(Op::Remove),
    ]
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    // Style A: lockstep loop. Compares every return value against the model.
    #[test]
    fn model_kv_store_lockstep(ops in prop::collection::vec(op(), 0..64)) {
        let mut real = KvStore::new();
        let mut model = BTreeMap::new();
        for op in ops {
            match op {
                Op::Insert(k, v) => prop_assert_eq!(real.insert(k.clone(), v), model.insert(k, v)),
                Op::Get(k) => prop_assert_eq!(real.get(&k), model.get(&k).copied()),
                Op::Remove(k) => prop_assert_eq!(real.remove(&k), model.remove(&k)),
            }
            prop_assert_eq!(real.len(), model.len());
            prop_assert_eq!(real.is_empty(), model.is_empty());
        }
    }
}

// Style B: proptest-state-machine, for when valid transitions depend on the state.
#[derive(Clone, Debug)]
enum Transition {
    Insert(String, i64),
    RemoveExisting(String),
}

struct Reference;

impl ReferenceStateMachine for Reference {
    type State = BTreeMap<String, i64>;
    type Transition = Transition;

    fn init_state() -> BoxedStrategy<Self::State> {
        Just(BTreeMap::new()).boxed()
    }

    fn transitions(state: &Self::State) -> BoxedStrategy<Self::Transition> {
        let insert = (key(), any::<i64>()).prop_map(|(k, v)| Transition::Insert(k, v));
        if state.is_empty() {
            insert.boxed()
        } else {
            let keys: Vec<String> = state.keys().cloned().collect();
            prop_oneof![
                insert,
                proptest::sample::select(keys).prop_map(Transition::RemoveExisting),
            ]
            .boxed()
        }
    }

    // Shrinking can delete earlier inserts, so re-check that the key still exists.
    fn preconditions(state: &Self::State, transition: &Self::Transition) -> bool {
        match transition {
            Transition::RemoveExisting(k) => state.contains_key(k),
            Transition::Insert(..) => true,
        }
    }

    fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
        match transition {
            Transition::Insert(k, v) => {
                state.insert(k.clone(), *v);
            }
            Transition::RemoveExisting(k) => {
                state.remove(k);
            }
        }
        state
    }
}

struct KvStoreMachine;

impl StateMachineTest for KvStoreMachine {
    type SystemUnderTest = KvStore;
    type Reference = Reference;

    fn init_test(_ref_state: &<Self::Reference as ReferenceStateMachine>::State) -> Self::SystemUnderTest {
        KvStore::new()
    }

    // `ref_state` already has `transition` applied: assert post-conditions here.
    fn apply(
        mut sut: Self::SystemUnderTest,
        ref_state: &<Self::Reference as ReferenceStateMachine>::State,
        transition: Transition,
    ) -> Self::SystemUnderTest {
        match transition {
            Transition::Insert(k, v) => {
                sut.insert(k.clone(), v);
                assert_eq!(sut.get(&k), Some(v));
            }
            Transition::RemoveExisting(k) => {
                assert!(sut.remove(&k).is_some(), "key {k} should have existed");
                assert_eq!(sut.get(&k), None);
            }
        }
        assert_eq!(sut.len(), ref_state.len());
        sut
    }

    fn check_invariants(
        sut: &Self::SystemUnderTest,
        ref_state: &<Self::Reference as ReferenceStateMachine>::State,
    ) {
        for (k, v) in ref_state {
            assert_eq!(sut.get(k), Some(*v));
        }
        // Added after cargo-mutants reported `KvStore::is_empty` mutants as MISSED.
        assert_eq!(sut.is_empty(), ref_state.is_empty());
    }
}

prop_state_machine! {
    #![proptest_config(common::proptest_config(128))]

    #[test]
    fn model_kv_store_state_machine(sequential 1..40 => KvStoreMachine);
}
