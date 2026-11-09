//! The labels of a project as Plane's v1 API has them (issue #58), against the memory adapters: what a read shows, how a label is made,
//! changed and deleted, and the order in which the pin checks things.
//!
//! What must hold: a read shows the live labels, newest first, and none of an archived project; a change and a deletion are looked up
//! the same way (a label of an archived project is not found, which a state is); a new label goes after the greatest live one by 10000;
//! a name that another label has is refused naming it; an integration key is refused only when both halves are named, and a change
//! leaves out the label itself and names the label it changes; a parent is a live label of the same project; and a deletion takes the
//! labels under it and the work items' hold on them.
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
use proptest::prelude::*;
use rstest::rstest;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

/// A project of workspace A made by `AdminA` (so its admin), and the scope that lets them write its labels.
fn project(w: &World, identifier: &str) -> (ProjectId, Scope) {
    let admin = w.principal(Actor::AdminA);
    let ws = WorkspaceScope::mint(w.ws_a, admin.user_id, Role::Admin);
    let patch = ProjectPatch {
        name: Some(format!("Labels {identifier}")),
        identifier: Some(identifier.into()),
        ..ProjectPatch::default()
    };
    let id = block_on(create_project(&w.app, &ws, patch)).unwrap().listing.project.id;
    let scope = block_on(authorize(&w.app, &admin, &w.slug_a, id, Permission::WriteLabel)).unwrap();
    (id, scope)
}

fn named(name: &str) -> LabelPatch {
    LabelPatch { name: Some(name.into()), ..LabelPatch::default() }
}

fn make(w: &World, scope: &Scope, name: &str) -> LabelRow {
    block_on(create_label(&w.app, scope, named(name))).unwrap()
}

fn live(w: &World, scope: &Scope) -> Vec<LabelRow> {
    block_on(w.app.ports.db.project_labels(scope)).unwrap()
}

fn names(w: &World, scope: &Scope) -> Vec<String> {
    let mut labels = live(w, scope);
    labels.sort_by_key(|l| l.sort_order.to_bits());
    labels.into_iter().map(|l| l.name).collect()
}

/// A work item of the project that carries `labels`, as the slice makes one and the compat attributes name the labels.
fn put_item(w: &World, scope: &Scope, n: u128, labels: &[Uuid]) -> WorkItemId {
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
        created_by: scope.user_id(),
        created_at: now,
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    };
    let mut tx = block_on(w.app.ports.db.begin(scope)).unwrap();
    block_on(tx.insert_work_item(scope, &item)).unwrap();
    let attrs = WorkItemAttrs { label_ids: labels.to_vec(), ..WorkItemAttrs::bare(now) };
    block_on(tx.insert_work_item_attrs(scope, item.id, &attrs)).unwrap();
    block_on(tx.commit()).unwrap();
    item.id
}

fn held(w: &World, scope: &Scope, item: WorkItemId) -> Vec<Uuid> {
    block_on(w.app.ports.db.work_item_row(scope, item)).unwrap().unwrap().attrs.label_ids
}

fn archive(w: &World, scope: &Scope, archived: bool) {
    let admin = WorkspaceScope::mint(w.ws_a, scope.user_id(), Role::Admin);
    block_on(set_project_archived(&w.app, &admin, scope.project_id(), archived)).unwrap();
}

// --- reading ------------------------------------------------------------------------------------------------------------------

#[test]
fn a_read_shows_the_live_labels_newest_first_and_none_of_an_archived_project() {
    let w = world();
    let (_, scope) = project(&w, "LBA");
    // the clock of this world is fixed, so these are made in one instant: the one placed last comes first
    let (bug, feature, docs) = (make(&w, &scope, "bug"), make(&w, &scope, "feature"), make(&w, &scope, "docs"));
    let shown = |w: &World| -> Vec<String> {
        block_on(visible_labels(&w.app, &scope)).unwrap().into_iter().map(|l| l.name).collect()
    };
    assert_eq!(shown(&w), ["docs", "feature", "bug"]);
    assert_eq!(block_on(visible_label(&w.app, &scope, feature.id)).unwrap(), feature);
    // one that is nowhere, and one of another project, are not found
    let (_, other) = project(&w, "LBB");
    let theirs = make(&w, &other, "bug");
    for missing in [uuid(0xBAD), theirs.id] {
        assert!(matches!(block_on(visible_label(&w.app, &scope, missing)), Err(LabelError::NotFound)));
    }
    // a deleted label is not shown
    block_on(delete_label(&w.app, &scope, docs.id)).unwrap();
    assert_eq!(shown(&w), ["feature", "bug"]);

    // and an archived project shows none, to a read, a change or a deletion alike
    archive(&w, &scope, true);
    assert!(shown(&w).is_empty());
    assert!(matches!(block_on(visible_label(&w.app, &scope, bug.id)), Err(LabelError::NotFound)));
    assert!(matches!(block_on(change_label(&w.app, &scope, bug.id, named("x"))), Err(LabelError::NotFound)));
    assert!(matches!(block_on(delete_label(&w.app, &scope, bug.id)), Err(LabelError::NotFound)));
    assert_eq!(names(&w, &scope), ["bug", "feature"], "and nothing was changed or deleted");
    // a creation does not look at the archive, as the pin's does not
    block_on(create_label(&w.app, &scope, named("ops"))).unwrap();
    archive(&w, &scope, false);
    assert_eq!(shown(&w), ["ops", "feature", "bug"]);
}

// --- making -------------------------------------------------------------------------------------------------------------------

#[test]
fn a_new_label_goes_after_the_greatest_live_one_by_ten_thousand() {
    let w = world();
    let (_, scope) = project(&w, "LBA");
    let first = block_on(create_label(&w.app, &scope, LabelPatch { sort_order: Some(7.0), ..named("a") })).unwrap();
    assert_eq!(first.sort_order, 7.0, "the first keeps the place it asked for");
    let second = block_on(create_label(&w.app, &scope, LabelPatch { sort_order: Some(1.0), ..named("b") })).unwrap();
    assert_eq!(second.sort_order, 10007.0, "and the next goes after it, whatever it asked for");
    // a deleted label is not counted, and a change of place moves the next
    block_on(delete_label(&w.app, &scope, second.id)).unwrap();
    assert_eq!(make(&w, &scope, "c").sort_order, 10007.0);
    // a change of place moves the next: `a` at 500 is now the greatest, `c` having moved to 50
    let c = live(&w, &scope).into_iter().find(|l| l.name == "c").unwrap();
    let place = |sort_order: f64| LabelPatch { sort_order: Some(sort_order), ..LabelPatch::default() };
    block_on(change_label(&w.app, &scope, first.id, place(500.0))).unwrap();
    block_on(change_label(&w.app, &scope, c.id, place(50.0))).unwrap();
    assert_eq!(make(&w, &scope, "d").sort_order, 10500.0);
}

#[test]
fn a_new_label_has_what_the_pin_gives_a_label_that_says_only_its_name() {
    let w = world();
    let (_, scope) = project(&w, "LBA");
    let label = make(&w, &scope, "bug");
    assert_eq!(
        (label.color.as_str(), label.description.as_str(), label.parent_id, label.sort_order),
        ("", "", None, 65535.0)
    );
    assert_eq!((label.created_by, label.updated_by), (Some(scope.user_id()), None));
    assert_eq!((label.workspace_id, label.project_id), (scope.workspace_id(), scope.project_id()));
    assert_eq!((label.created_at, label.updated_at), (w.app.ports.clock.now(), w.app.ports.clock.now()));
    assert_eq!((label.external_source, label.external_id), (None, None));
    // and it needs a name
    let nameless = block_on(create_label(&w.app, &scope, LabelPatch::default()));
    assert!(matches!(nameless, Err(LabelError::Core(CoreError::Invalid(_)))), "{nameless:?}");
    assert_eq!(live(&w, &scope).len(), 1);
}

#[test]
fn a_name_that_another_label_has_is_refused_naming_it_and_a_deleted_labels_name_is_free() {
    let w = world();
    let (_, scope) = project(&w, "LBA");
    let bug = make(&w, &scope, "bug");
    let feature = make(&w, &scope, "feature");
    let got = block_on(create_label(&w.app, &scope, named("bug")));
    assert!(matches!(got, Err(LabelError::NameTaken { id: Some(id) }) if id == bug.id), "{got:?}");
    // a name is exact, and another project may have it
    let upper = make(&w, &scope, "Bug");
    let (_, other) = project(&w, "LBB");
    make(&w, &other, "bug");
    // a change onto a name another label has is refused, and onto its own is not
    let onto = block_on(change_label(&w.app, &scope, feature.id, named("bug")));
    assert!(matches!(onto, Err(LabelError::NameTaken { id: Some(id) }) if id == bug.id), "{onto:?}");
    // and the label named is the one that has the name, not the first of the others: `Bug` is the last of the three
    let onto = block_on(change_label(&w.app, &scope, upper.id, named("feature")));
    assert!(matches!(onto, Err(LabelError::NameTaken { id: Some(id) }) if id == feature.id), "{onto:?}");
    block_on(change_label(&w.app, &scope, feature.id, named("feature"))).unwrap();
    // a deleted label's name is free, and the label that has it now is another row
    block_on(delete_label(&w.app, &scope, bug.id)).unwrap();
    let again = make(&w, &scope, "bug");
    assert_ne!(again.id, bug.id);
}

// --- integration keys ---------------------------------------------------------------------------------------------------------

/// The pin refuses a key on a creation only when the request names both halves, and neither is empty.
#[rstest]
#[case::the_same_key(Some("gh"), Some("1"), true)]
#[case::another_source(Some("jira"), Some("1"), false)]
#[case::another_id(Some("gh"), Some("2"), false)]
#[case::no_source(None, Some("1"), false)]
#[case::no_id(Some("gh"), None, false)]
#[case::an_empty_source(Some(""), Some("1"), false)]
#[case::an_empty_id(Some("gh"), Some(""), false)]
#[case::neither(None, None, false)]
fn a_creation_is_refused_for_a_key_another_label_has_only_when_both_halves_are_named(
    #[case] source: Option<&str>,
    #[case] id: Option<&str>,
    #[case] refused: bool,
) {
    let w = world();
    let (_, scope) = project(&w, "LBA");
    let keyed =
        LabelPatch { external_source: Some(Some("gh".into())), external_id: Some(Some("1".into())), ..named("keyed") };
    let holder = block_on(create_label(&w.app, &scope, keyed)).unwrap();
    let wants = LabelPatch {
        external_source: source.map(|s| Some(s.into())),
        external_id: id.map(|i| Some(i.into())),
        ..named("another")
    };
    let got = block_on(create_label(&w.app, &scope, wants));
    if refused {
        assert!(matches!(got, Err(LabelError::ExternalTaken { id }) if id == holder.id), "{got:?}");
        assert_eq!(live(&w, &scope).len(), 1, "nothing was made");
    } else {
        assert!(got.is_ok(), "{got:?}");
    }
}

/// A change refuses a key that another label has, naming the label it changes (not the one that has the key); both halves must be named
/// in the request itself (the label's own source does not fill in a missing one); and a label may keep, or be given again, its own key.
#[test]
fn a_change_is_refused_for_a_key_another_label_has_naming_the_label_it_changes() {
    let w = world();
    let (_, scope) = project(&w, "LBA");
    let key = |s: &str, i: &str| LabelPatch {
        external_source: Some(Some(s.into())),
        external_id: Some(Some(i.into())),
        ..LabelPatch::default()
    };
    let holder = block_on(create_label(&w.app, &scope, LabelPatch { ..key("gh", "1") })).map(|_| ()).err();
    assert!(holder.is_some(), "a label needs a name");
    let holder =
        block_on(create_label(&w.app, &scope, LabelPatch { name: Some("holder".into()), ..key("gh", "1") })).unwrap();
    let other = make(&w, &scope, "other");
    let got = block_on(change_label(&w.app, &scope, other.id, key("gh", "1")));
    assert!(matches!(got, Err(LabelError::ExternalTaken { id }) if id == other.id), "{got:?}");
    // half a key is not checked
    let half = LabelPatch { external_id: Some(Some("1".into())), ..LabelPatch::default() };
    assert_eq!(block_on(change_label(&w.app, &scope, other.id, half)).unwrap().external_id.as_deref(), Some("1"));
    // the holder is given its own key again, and then changed to another
    block_on(change_label(&w.app, &scope, holder.id, key("gh", "1"))).unwrap();
    assert!(block_on(change_label(&w.app, &scope, holder.id, key("gh", "2"))).is_ok());
    // and a key that nobody has is free
    assert!(block_on(change_label(&w.app, &scope, other.id, key("gh", "1"))).is_ok());
}

// --- parents ------------------------------------------------------------------------------------------------------------------

#[test]
fn a_parent_is_a_live_label_of_the_same_project() {
    let w = world();
    let (_, scope) = project(&w, "LBA");
    let (_, other) = project(&w, "LBB");
    let parent = make(&w, &scope, "parent");
    let foreign = make(&w, &other, "foreign");
    let doomed = make(&w, &scope, "doomed");
    block_on(delete_label(&w.app, &scope, doomed.id)).unwrap();
    let under = |p: Uuid| LabelPatch { parent: Some(Some(p)), ..named("child") };

    let child = block_on(create_label(&w.app, &scope, under(parent.id))).unwrap();
    assert_eq!(child.parent_id, Some(parent.id));
    assert!(block_on(is_label_of_project(&w.app, &scope, parent.id)).unwrap());
    // none of these is a label of this project: another project's, a deleted one and one that is nowhere
    for not_here in [foreign.id, doomed.id, uuid(0xBAD)] {
        assert!(!block_on(is_label_of_project(&w.app, &scope, not_here)).unwrap());
        let got = block_on(create_label(
            &w.app,
            &scope,
            LabelPatch { name: Some(format!("c{not_here}")), ..under(not_here) },
        ));
        assert!(matches!(got, Err(LabelError::NoSuchParent(p)) if p == not_here), "{got:?}");
        let got = block_on(change_label(
            &w.app,
            &scope,
            child.id,
            LabelPatch { parent: Some(Some(not_here)), ..LabelPatch::default() },
        ));
        assert!(matches!(got, Err(LabelError::NoSuchParent(p)) if p == not_here), "{got:?}");
    }
    assert_eq!(live(&w, &scope).len(), 2, "no label was made, and the child still has its parent");
    // a change can clear the parent (null), and can name the label itself, as the pin allows
    let cleared =
        block_on(change_label(&w.app, &scope, child.id, LabelPatch { parent: Some(None), ..LabelPatch::default() }))
            .unwrap();
    assert_eq!(cleared.parent_id, None);
    let itself = block_on(change_label(
        &w.app,
        &scope,
        child.id,
        LabelPatch { parent: Some(Some(child.id)), ..LabelPatch::default() },
    ))
    .unwrap();
    assert_eq!(itself.parent_id, Some(child.id));
}

// --- changing -----------------------------------------------------------------------------------------------------------------

#[test]
fn a_change_sets_what_it_names_records_who_and_when_and_keeps_the_rest() {
    let w = world();
    let (_, scope) = project(&w, "LBA");
    let parent = make(&w, &scope, "parent");
    let before = make(&w, &scope, "bug");
    let patch = LabelPatch {
        name: Some("defect".into()),
        description: Some("d".into()),
        color: Some("#abcdef".into()),
        sort_order: Some(123.5),
        parent: Some(Some(parent.id)),
        external_source: Some(Some("gh".into())),
        external_id: Some(Some("7".into())),
    };
    let after = block_on(change_label(&w.app, &scope, before.id, patch)).unwrap();
    assert_eq!(
        (after.name.as_str(), after.description.as_str(), after.color.as_str(), after.sort_order, after.parent_id),
        ("defect", "d", "#abcdef", 123.5, Some(parent.id))
    );
    assert_eq!((after.external_source.as_deref(), after.external_id.as_deref()), (Some("gh"), Some("7")));
    assert_eq!((after.updated_by, after.updated_at), (Some(scope.user_id()), w.app.ports.clock.now()));
    assert_eq!((after.id, after.created_by, after.created_at), (before.id, before.created_by, before.created_at));
    // what is returned is what is stored
    assert_eq!(live(&w, &scope).into_iter().find(|l| l.id == before.id), Some(after));
    // naming nothing changes nothing but who and when, and null clears a key
    let again = block_on(change_label(&w.app, &scope, before.id, LabelPatch::default())).unwrap();
    assert_eq!((again.name.as_str(), again.sort_order), ("defect", 123.5));
    let cleared = LabelPatch { external_id: Some(None), external_source: Some(None), ..LabelPatch::default() };
    let cleared = block_on(change_label(&w.app, &scope, before.id, cleared)).unwrap();
    assert_eq!((cleared.external_source, cleared.external_id), (None, None));
    // a label that is nowhere, one of another project and a deleted one are not found
    let (_, other) = project(&w, "LBB");
    let theirs = make(&w, &other, "theirs");
    block_on(delete_label(&w.app, &scope, before.id)).unwrap();
    for missing in [uuid(0xBAD), theirs.id, before.id] {
        let got = block_on(change_label(&w.app, &scope, missing, named("x")));
        assert!(matches!(got, Err(LabelError::NotFound)), "{got:?}");
    }
}

// --- deleting -----------------------------------------------------------------------------------------------------------------

#[test]
fn a_deletion_takes_the_labels_under_the_label_and_the_work_items_hold_on_them() {
    let w = world();
    let (_, scope) = project(&w, "LBA");
    let parent = make(&w, &scope, "parent");
    let child =
        block_on(create_label(&w.app, &scope, LabelPatch { parent: Some(Some(parent.id)), ..named("child") })).unwrap();
    let grandchild =
        block_on(create_label(&w.app, &scope, LabelPatch { parent: Some(Some(child.id)), ..named("grandchild") }))
            .unwrap();
    let sibling = make(&w, &scope, "sibling");
    let mut both = vec![parent.id, sibling.id];
    both.sort();
    let mut deep = vec![grandchild.id, sibling.id];
    deep.sort();
    let (a, b) = (put_item(&w, &scope, 1, &both), put_item(&w, &scope, 2, &deep));

    block_on(delete_label(&w.app, &scope, child.id)).unwrap();
    assert_eq!(names(&w, &scope), ["parent", "sibling"]);
    assert_eq!(held(&w, &scope, b), [sibling.id]);
    assert_eq!(held(&w, &scope, a), both);
    block_on(delete_label(&w.app, &scope, parent.id)).unwrap();
    assert_eq!(held(&w, &scope, a), [sibling.id]);
    // a second deletion and a label that is nowhere are not found
    for missing in [parent.id, uuid(0xBAD)] {
        assert!(matches!(block_on(delete_label(&w.app, &scope, missing)), Err(LabelError::NotFound)));
    }
}

// --- the place, as a model ----------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Step {
    Make,
    Delete(usize),
    Move(usize, u32),
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// Whatever labels are made, deleted and moved, a new label's place is the greatest live place plus 10000 (the first, 65535), and
    /// the live labels are exactly those the model has, each at the place the model gives it.
    #[test]
    fn prop_a_new_labels_place_follows_the_greatest_live_one(
        steps in prop::collection::vec(
            prop_oneof![
                3 => Just(Step::Make),
                1 => (0..8usize).prop_map(Step::Delete),
                1 => (0..8usize, 0..1_000_000u32).prop_map(|(i, p)| Step::Move(i, p)),
            ],
            1..24,
        ),
    ) {
        let w = world();
        let (_, scope) = project(&w, "LBP");
        let mut model: Vec<(Uuid, f64)> = Vec::new();
        for (n, step) in steps.into_iter().enumerate() {
            match step {
                Step::Make => {
                    let want = model.iter().map(|m| m.1).reduce(f64::max).map_or(65535.0, |g| g + 10000.0);
                    let made = block_on(create_label(&w.app, &scope, named(&format!("l{n}")))).unwrap();
                    prop_assert_eq!(made.sort_order, want);
                    model.push((made.id, want));
                }
                Step::Delete(i) if !model.is_empty() => {
                    let (id, _) = model.remove(i % model.len());
                    prop_assert!(block_on(delete_label(&w.app, &scope, id)).is_ok());
                }
                Step::Move(i, place) if !model.is_empty() => {
                    let at = i % model.len();
                    let place = f64::from(place);
                    let patch = LabelPatch { sort_order: Some(place), ..LabelPatch::default() };
                    prop_assert!(block_on(change_label(&w.app, &scope, model[at].0, patch)).is_ok());
                    model[at].1 = place;
                }
                _ => {}
            }
        }
        let mut got: Vec<(Uuid, u64)> = live(&w, &scope).into_iter().map(|l| (l.id, l.sort_order.to_bits())).collect();
        let mut want: Vec<(Uuid, u64)> = model.into_iter().map(|(id, p)| (id, p.to_bits())).collect();
        got.sort();
        want.sort();
        prop_assert_eq!(got, want);
    }
}
