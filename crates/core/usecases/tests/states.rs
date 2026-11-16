//! The states of a project as Plane's v1 API has them (issue #58), against the memory adapters: what a read shows, how a state is made,
//! changed and deleted, and the order in which the pin checks things.
//!
//! What must hold: a read shows the live states that are not the intake one and none of an archived project; a new state goes after the
//! greatest one that is not the intake state and refuses the group `triage`; a name or an integration key that another state has is
//! refused naming that state (a clash with the intake state is the pin's accidental failure); a change finds its state before it checks
//! anything, works on an archived project, and names the state it changes when a key is taken; the default is one state's at a time; and
//! a state is deleted unless it is the default or has work items in it, in the pin's sense.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use common::*;
use futures_executor::block_on;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

/// A project of workspace A made by `AdminA` (so its admin), with the pin's six states; and the scope that lets them write its states.
fn project(w: &World) -> (ProjectId, Scope) {
    let admin = w.principal(Actor::AdminA);
    let ws = WorkspaceScope::mint(w.ws_a, admin.user_id, Role::Admin);
    let patch = ProjectPatch { name: Some("States".into()), identifier: Some("STA".into()), ..ProjectPatch::default() };
    let id = block_on(create_project(&w.app, &ws, patch)).unwrap().listing.project.id;
    let scope = block_on(authorize(&w.app, &admin, &w.slug_a, id, Permission::WriteState)).unwrap();
    (id, scope)
}

fn named(name: &str) -> StatePatch {
    StatePatch { name: Some(name.into()), color: Some("#123456".into()), ..StatePatch::default() }
}

fn state(w: &World, scope: &Scope, name: &str) -> StateRow {
    block_on(w.app.ports.db.project_states(scope)).unwrap().into_iter().find(|s| s.name == name).unwrap()
}

fn defaults(w: &World, scope: &Scope) -> Vec<String> {
    block_on(w.app.ports.db.project_states(scope))
        .unwrap()
        .into_iter()
        .filter(|s| s.is_default)
        .map(|s| s.name)
        .collect()
}

/// A work item in `state` of the project, as the slice makes one and the compat attributes name the state.
fn put_item(w: &World, scope: &Scope, n: u128, state: Uuid, archived: bool) {
    let now = w.app.ports.clock.now();
    let item = WorkItem {
        id: WorkItemId(uuid(0x9000 + n)),
        workspace_id: scope.workspace_id(),
        project_id: scope.project_id(),
        sequence_id: n as i64,
        sort_order: 1,
        name: format!("w{n}"),
        description: None,
        external_source: None,
        external_id: None,
        created_by: Some(scope.user_id()),
        created_at: now,
        archived_at: archived.then_some(now),
        is_draft: false,
        deleted_at: None,
    };
    let mut tx = block_on(w.app.ports.db.begin(scope)).unwrap();
    block_on(tx.insert_work_item(scope, &item)).unwrap();
    let attrs = WorkItemAttrs { state_id: Some(state), ..WorkItemAttrs::bare(now) };
    block_on(tx.insert_work_item_attrs(scope, item.id, &attrs)).unwrap();
    block_on(tx.commit()).unwrap();
}

// --- reading ------------------------------------------------------------------------------------------------------------------

#[test]
fn a_read_shows_the_live_states_that_are_not_the_intake_one_and_none_of_an_archived_project() {
    let w = world();
    let (id, scope) = project(&w);
    let names = |states: Vec<StateRow>| states.into_iter().map(|s| s.name).collect::<Vec<_>>();
    let seen = block_on(visible_states(&w.app, &scope)).unwrap();
    assert_eq!(names(seen), ["Backlog", "Todo", "In Progress", "Done", "Cancelled"], "the intake state is not shown");
    let todo = state(&w, &scope, "Todo");
    assert_eq!(block_on(visible_state(&w.app, &scope, todo.id)).unwrap(), todo);
    // the intake state and one that is nowhere are not found
    let triage = block_on(w.app.ports.db.triage_state(&scope)).unwrap().unwrap();
    for hidden in [triage.id, uuid(0xBAD)] {
        assert!(matches!(block_on(visible_state(&w.app, &scope, hidden)), Err(StateError::NotFound)));
    }
    // a deleted state is not shown
    block_on(delete_state(&w.app, &scope, todo.id)).unwrap();
    assert_eq!(block_on(visible_states(&w.app, &scope)).unwrap().len(), 4);

    // and an archived project shows none
    let admin = WorkspaceScope::mint(w.ws_a, scope.user_id(), Role::Admin);
    block_on(set_project_archived(&w.app, &admin, id, true)).unwrap();
    assert!(block_on(visible_states(&w.app, &scope)).unwrap().is_empty());
    assert!(matches!(block_on(visible_state(&w.app, &scope, state(&w, &scope, "Done").id)), Err(StateError::NotFound)));
}

// --- making -------------------------------------------------------------------------------------------------------------------

#[test]
fn a_new_state_goes_after_the_greatest_one_that_is_not_the_intake_state() {
    let w = world();
    let (_, scope) = project(&w);
    // 15000 to 55000, and the intake state at 65000 that does not count
    let a = block_on(create_state(&w.app, &scope, StatePatch { sequence: Some(1.0), ..named("Review") })).unwrap();
    assert_eq!(a.sequence, 70000.0, "whatever place it asked for");
    let b = block_on(create_state(&w.app, &scope, named("QA"))).unwrap();
    assert_eq!(b.sequence, 85000.0);
    // what it carries: the project, the maker, the time, the group `backlog` and no integration key
    assert_eq!((a.project_id, a.workspace_id), (scope.project_id(), scope.workspace_id()));
    assert_eq!((a.created_by, a.updated_by), (Some(scope.user_id()), None));
    assert_eq!((a.created_at, a.updated_at), (w.app.ports.clock.now(), w.app.ports.clock.now()));
    assert_eq!(
        (a.group, a.is_default, a.description.as_str(), a.color.as_str()),
        (StateGroup::Backlog, false, "", "#123456")
    );
    assert_eq!((a.external_source, a.external_id), (None, None));
    // it is the one the lists show, and its id is the id generator's
    assert_eq!(block_on(visible_state(&w.app, &scope, a.id)).unwrap().name, "Review");
    // a group and a description it names are kept
    let c = StatePatch { group: Some(StateGroup::Started), description: Some("d".into()), ..named("Doing") };
    let c = block_on(create_state(&w.app, &scope, c)).unwrap();
    assert_eq!((c.group, c.description.as_str()), (StateGroup::Started, "d"));
}

#[test]
fn a_new_state_needs_a_name_and_a_colour_and_is_never_a_triage_state() {
    let w = world();
    let (_, scope) = project(&w);
    let before = block_on(w.app.ports.db.project_states(&scope)).unwrap().len();
    for patch in [
        StatePatch::default(),
        StatePatch { name: Some("n".into()), ..StatePatch::default() },
        StatePatch { color: Some("#000".into()), ..StatePatch::default() },
    ] {
        let got = block_on(create_state(&w.app, &scope, patch));
        assert!(matches!(got, Err(StateError::Core(CoreError::Invalid(_)))), "{got:?}");
    }
    let triage = StatePatch { group: Some(StateGroup::Triage), ..named("Intake") };
    assert!(matches!(block_on(create_state(&w.app, &scope, triage)), Err(StateError::TriageGroup)));
    assert_eq!(block_on(w.app.ports.db.project_states(&scope)).unwrap().len(), before, "a refusal writes nothing");
}

/// A name another state has is refused naming that state; a deleted state's is free; another case is another name; and a clash with
/// the intake state, which the pin does not show, is the pin's failure (it looks the state up among those it shows, finds none and
/// fails), which is an `Internal` here.
#[test]
fn a_name_that_another_state_has_is_refused_naming_it() {
    let w = world();
    let (_, scope) = project(&w);
    let todo = state(&w, &scope, "Todo");
    match block_on(create_state(&w.app, &scope, named("Todo"))) {
        Err(StateError::NameTaken { id }) => assert_eq!(id, Some(todo.id)),
        other => panic!("{other:?}"),
    }
    assert!(block_on(create_state(&w.app, &scope, named("todo"))).is_ok());
    block_on(delete_state(&w.app, &scope, todo.id)).unwrap();
    assert!(block_on(create_state(&w.app, &scope, named("Todo"))).is_ok());
    let got = block_on(create_state(&w.app, &scope, named("Triage")));
    assert!(matches!(got, Err(StateError::Core(CoreError::Internal(_)))), "{got:?}");
}

/// The pin refuses a key only when the request names both halves of it, and the state it names is the one that has it.
#[test]
fn an_integration_key_that_another_state_has_is_refused_when_both_halves_are_named() {
    let w = world();
    let (_, scope) = project(&w);
    let keyed = |name: &str, source: Option<&str>, key: Option<&str>| StatePatch {
        external_source: source.map(|s| Some(s.to_string())),
        external_id: key.map(|k| Some(k.to_string())),
        ..named(name)
    };
    let first = block_on(create_state(&w.app, &scope, keyed("One", Some("gh"), Some("1")))).unwrap();
    assert_eq!((first.external_source.as_deref(), first.external_id.as_deref()), (Some("gh"), Some("1")));
    match block_on(create_state(&w.app, &scope, keyed("Two", Some("gh"), Some("1")))) {
        Err(StateError::ExternalTaken { id }) => assert_eq!(id, first.id),
        other => panic!("{other:?}"),
    }
    // another source, another key, or only one half named: not refused
    assert!(block_on(create_state(&w.app, &scope, keyed("Three", Some("jira"), Some("1")))).is_ok());
    assert!(block_on(create_state(&w.app, &scope, keyed("Four", Some("gh"), Some("2")))).is_ok());
    assert!(block_on(create_state(&w.app, &scope, keyed("Five", None, Some("1")))).is_ok());
    assert!(block_on(create_state(&w.app, &scope, keyed("Six", Some("gh"), None))).is_ok());
    // and the refusal comes before the write: nothing was made by it
    assert!(block_on(w.app.ports.db.project_states(&scope)).unwrap().iter().all(|s| s.name != "Two"));
}

// --- the default --------------------------------------------------------------------------------------------------------------

#[test]
fn the_default_is_one_state_at_a_time_and_a_change_can_leave_the_project_with_none() {
    let w = world();
    let (_, scope) = project(&w);
    assert_eq!(defaults(&w, &scope), ["Backlog"]);
    block_on(create_state(&w.app, &scope, named("Plain"))).unwrap();
    assert_eq!(defaults(&w, &scope), ["Backlog"]);
    block_on(create_state(&w.app, &scope, StatePatch { is_default: Some(true), ..named("Review") })).unwrap();
    assert_eq!(defaults(&w, &scope), ["Review"]);
    let todo = state(&w, &scope, "Todo");
    block_on(change_state(&w.app, &scope, todo.id, StatePatch { is_default: Some(true), ..StatePatch::default() }))
        .unwrap();
    assert_eq!(defaults(&w, &scope), ["Todo"]);
    block_on(change_state(&w.app, &scope, todo.id, StatePatch { is_default: Some(false), ..StatePatch::default() }))
        .unwrap();
    assert!(defaults(&w, &scope).is_empty());
}

// --- changing -----------------------------------------------------------------------------------------------------------------

#[test]
fn a_change_sets_what_it_names_records_who_and_when_and_keeps_the_rest() {
    let w = world();
    let (_, scope) = project(&w);
    let before = state(&w, &scope, "Todo");
    let patch = StatePatch {
        name: Some("Renamed".into()),
        description: Some("d".into()),
        color: Some("#abcdef".into()),
        sequence: Some(123.5),
        group: Some(StateGroup::Completed),
        external_source: Some(Some("gh".into())),
        external_id: Some(Some("7".into())),
        ..StatePatch::default()
    };
    let after = block_on(change_state(&w.app, &scope, before.id, patch)).unwrap();
    assert_eq!(
        (after.name.as_str(), after.description.as_str(), after.color.as_str(), after.sequence, after.group),
        ("Renamed", "d", "#abcdef", 123.5, StateGroup::Completed)
    );
    assert_eq!((after.external_source.as_deref(), after.external_id.as_deref()), (Some("gh"), Some("7")));
    assert_eq!((after.updated_by, after.updated_at), (Some(scope.user_id()), w.app.ports.clock.now()));
    assert_eq!(
        (after.id, after.created_by, after.created_at, after.is_default),
        (before.id, before.created_by, before.created_at, false)
    );
    // naming nothing changes nothing but who and when, and null clears a key
    let again = block_on(change_state(&w.app, &scope, before.id, StatePatch::default())).unwrap();
    assert_eq!((again.name.as_str(), again.sequence), ("Renamed", 123.5));
    let cleared = StatePatch { external_id: Some(None), external_source: Some(None), ..StatePatch::default() };
    let cleared = block_on(change_state(&w.app, &scope, before.id, cleared)).unwrap();
    assert_eq!((cleared.external_source, cleared.external_id), (None, None));
}

/// The pin finds the state first (a state that is nowhere is not found before any check of the request), and only then reads the
/// request: the group `triage` is refused after that.
#[test]
fn a_change_finds_its_state_before_it_checks_anything() {
    let w = world();
    let (_, scope) = project(&w);
    let triage_patch = StatePatch { group: Some(StateGroup::Triage), ..StatePatch::default() };
    let missing = block_on(change_state(&w.app, &scope, uuid(0xBAD), triage_patch.clone()));
    assert!(matches!(missing, Err(StateError::NotFound)), "{missing:?}");
    let todo = state(&w, &scope, "Todo");
    let got = block_on(change_state(&w.app, &scope, todo.id, triage_patch));
    assert!(matches!(got, Err(StateError::TriageGroup)), "{got:?}");
    // the intake state is not found either, and nothing was changed
    let intake = block_on(w.app.ports.db.triage_state(&scope)).unwrap().unwrap();
    assert!(matches!(
        block_on(change_state(&w.app, &scope, intake.id, StatePatch::default())),
        Err(StateError::NotFound)
    ));
    assert_eq!(state(&w, &scope, "Todo").group, StateGroup::Unstarted);
}

/// The pin's change does not look at whether the project is archived (its reads do).
#[test]
fn a_change_and_a_deletion_work_on_a_state_of_an_archived_project() {
    let w = world();
    let (id, scope) = project(&w);
    let admin = WorkspaceScope::mint(w.ws_a, scope.user_id(), Role::Admin);
    block_on(set_project_archived(&w.app, &admin, id, true)).unwrap();
    assert!(block_on(visible_states(&w.app, &scope)).unwrap().is_empty(), "a read shows none");
    let todo = state(&w, &scope, "Todo");
    let changed = block_on(change_state(
        &w.app,
        &scope,
        todo.id,
        StatePatch { name: Some("Still".into()), ..StatePatch::default() },
    ));
    assert_eq!(changed.unwrap().name, "Still");
    assert!(block_on(delete_state(&w.app, &scope, todo.id)).is_ok());
}

/// A key that is new is refused when another state has it with the source the request names (or else the state's own), and the answer
/// names the state that is being changed, not the one that has the key. A state keeps its own key.
#[test]
fn a_change_to_an_integration_key_that_another_state_has_names_the_state_being_changed() {
    let w = world();
    let (_, scope) = project(&w);
    let keyed =
        StatePatch { external_source: Some(Some("gh".into())), external_id: Some(Some("1".into())), ..named("Keyed") };
    let keyed = block_on(create_state(&w.app, &scope, keyed)).unwrap();
    let todo = state(&w, &scope, "Todo");
    let key = |source: Option<Option<&str>>, id: &str| StatePatch {
        external_source: source.map(|s| s.map(str::to_string)),
        external_id: Some(Some(id.to_string())),
        ..StatePatch::default()
    };
    // the source named, and the source the state has (none): the first is taken, the second is not a key anyone has
    match block_on(change_state(&w.app, &scope, todo.id, key(Some(Some("gh")), "1"))) {
        Err(StateError::ExternalTaken { id }) => {
            assert_eq!(id, todo.id, "the state being changed, not the one that has the key")
        }
        other => panic!("{other:?}"),
    }
    assert!(
        block_on(change_state(&w.app, &scope, todo.id, key(None, "1"))).is_ok(),
        "no source: the state's own, which is none"
    );
    // the state that has the key keeps it, and the same key with another source is another key
    assert!(block_on(change_state(&w.app, &scope, keyed.id, key(Some(Some("gh")), "1"))).is_ok());
    assert!(block_on(change_state(&w.app, &scope, keyed.id, key(Some(Some("jira")), "1"))).is_ok());
}

#[test]
fn a_change_cannot_take_a_name_another_state_has() {
    let w = world();
    let (_, scope) = project(&w);
    let done = state(&w, &scope, "Done");
    let got = block_on(change_state(
        &w.app,
        &scope,
        done.id,
        StatePatch { name: Some("Todo".into()), ..StatePatch::default() },
    ));
    assert!(matches!(got, Err(StateError::NameTaken { id: Some(_) })), "{got:?}");
    // its own name is not another's, and a name that only the intake state has is refused without a state to name
    assert!(block_on(change_state(
        &w.app,
        &scope,
        done.id,
        StatePatch { name: Some("Done".into()), ..StatePatch::default() }
    ))
    .is_ok());
    let got = block_on(change_state(
        &w.app,
        &scope,
        done.id,
        StatePatch { name: Some("Triage".into()), ..StatePatch::default() },
    ));
    assert!(matches!(got, Err(StateError::NameTaken { id: None })), "{got:?}");
    assert_eq!(state(&w, &scope, "Done").id, done.id, "nothing changed");
}

/// What a change and a deletion find: a live state that is not the intake one, whether or not the project is archived; any other is
/// not found, so a request about it is answered before its body is looked at.
#[test]
fn a_state_is_found_for_a_change_when_it_is_live_and_not_the_intake_one_whatever_the_archive_says() {
    let w = world();
    let (id, scope) = project(&w);
    let todo = state(&w, &scope, "Todo");
    assert_eq!(block_on(find_state(&w.app, &scope, todo.id)).unwrap(), todo);
    let admin = WorkspaceScope::mint(w.ws_a, scope.user_id(), Role::Admin);
    block_on(set_project_archived(&w.app, &admin, id, true)).unwrap();
    assert_eq!(block_on(find_state(&w.app, &scope, todo.id)).unwrap(), todo, "a read hides it; a change finds it");
    let intake = block_on(w.app.ports.db.triage_state(&scope)).unwrap().unwrap();
    block_on(delete_state(&w.app, &scope, todo.id)).unwrap();
    for not_found in [intake.id, todo.id, uuid(0xBAD)] {
        assert!(matches!(block_on(find_state(&w.app, &scope, not_found)), Err(StateError::NotFound)), "{not_found}");
    }
}

// --- deleting -----------------------------------------------------------------------------------------------------------------

#[test]
fn a_state_is_deleted_unless_it_is_the_default_or_has_work_items_in_the_pins_sense() {
    let w = world();
    let (_, scope) = project(&w);
    let backlog = state(&w, &scope, "Backlog");
    assert!(matches!(block_on(delete_state(&w.app, &scope, backlog.id)), Err(StateError::DefaultState)));

    // a work item that counts keeps its state; one that is archived does not
    let todo = state(&w, &scope, "Todo");
    put_item(&w, &scope, 1, todo.id, false);
    assert!(matches!(block_on(delete_state(&w.app, &scope, todo.id)), Err(StateError::NotEmpty)));
    let done = state(&w, &scope, "Done");
    put_item(&w, &scope, 2, done.id, true);
    assert!(block_on(delete_state(&w.app, &scope, done.id)).is_ok(), "an archived work item does not count");

    // what was refused deleted nothing, and what was deleted is gone and its name is free
    assert!(block_on(visible_state(&w.app, &scope, todo.id)).is_ok());
    assert!(matches!(block_on(visible_state(&w.app, &scope, done.id)), Err(StateError::NotFound)));
    assert!(matches!(block_on(delete_state(&w.app, &scope, done.id)), Err(StateError::NotFound)));
    assert!(block_on(create_state(&w.app, &scope, named("Done"))).is_ok());
    // and the intake state and a state that is nowhere are not there to delete
    let intake = block_on(w.app.ports.db.triage_state(&scope)).unwrap().unwrap();
    for hidden in [intake.id, uuid(0xBAD)] {
        assert!(matches!(block_on(delete_state(&w.app, &scope, hidden)), Err(StateError::NotFound)));
    }
}
