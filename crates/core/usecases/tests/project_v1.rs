//! A project as Plane's v1 API has it (issue #58): who may write one, what archiving and deleting do, and which projects a caller sees,
//! against the memory adapters.
//!
//! What must hold: a write on one project (a change, a deletion, archiving, unarchiving) is for a project admin, or a member of the
//! project who administers the workspace, and nobody else, a project that is not there included; a change names a default state that
//! is one of the project's own, and an archived project is not changed; archiving records when and by whom, and unarchiving clears it;
//! a deleted project leaves every read, and its name and key are free again; and a caller sees the projects they are in and the public
//! ones, whatever their role in the workspace (the internal API's rule differs, so a workspace admin does not see a secret project
//! they are not in, and a guest does see a public one).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::collections::BTreeSet;

use common::*;
use futures_executor::block_on;
use rstest::rstest;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

fn wscope(w: &World, who: Actor, role: Role) -> WorkspaceScope {
    WorkspaceScope::mint(w.ws_a, w.principal(who).user_id, role)
}

/// The scope the caller really has in workspace A (their own role, not one the test picks).
fn real_scope(w: &World, who: Actor) -> WorkspaceScope {
    block_on(authorize_workspace(&w.app, &w.principal(who), &w.slug_a)).unwrap()
}

/// The caller's role in project A, as `world()` seeds them: the three members of it and one who is in the workspace only.
fn project_role(who: Actor) -> Option<Role> {
    match who {
        Actor::AdminA => Some(Role::Admin),
        Actor::MemberA => Some(Role::Member),
        Actor::GuestA => Some(Role::Guest),
        _ => None,
    }
}

fn patch(name: &str, identifier: &str) -> ProjectPatch {
    ProjectPatch { name: Some(name.into()), identifier: Some(identifier.into()), ..ProjectPatch::default() }
}

// --- who may write ------------------------------------------------------------------------------------------------------

/// Plane's `ProjectBasePermission` for anything but a read or a creation: a project admin, or a member of the project who is an admin
/// of the workspace. The matrix is every kind of caller of project A against every role they could hold in the workspace; the
/// expectation is the rule, written out, not asked of the code.
#[rstest]
fn the_writer_is_a_project_admin_or_a_project_member_who_administers_the_workspace(
    #[values(Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA)] who: Actor,
    #[values(Role::Guest, Role::Member, Role::Admin)] ws_role: Role,
) {
    let w = world();
    let got = block_on(project_writer(&w.app, &wscope(&w, who, ws_role), w.project_a));
    let project = project_role(who);
    let allowed = project == Some(Role::Admin) || (project.is_some() && ws_role == Role::Admin);
    match (allowed, &got) {
        (true, Ok(d)) => assert_eq!(d.listing.project.id, w.project_a),
        (false, Err(ProjectError::Forbidden)) => {}
        _ => panic!("{who:?} (project {project:?}) as workspace {ws_role:?}: {got:?}"),
    }
}

/// The internal API tells a workspace admin that a project is not there; v1 does not: the pin looks for the caller's membership of it
/// and finds none, so a project that is not there is refused to everyone, an admin of the workspace included.
#[rstest]
fn a_project_that_is_not_there_has_no_writer(#[values(Role::Guest, Role::Member, Role::Admin)] ws_role: Role) {
    let w = world();
    let got = block_on(project_writer(&w.app, &wscope(&w, Actor::AdminA, ws_role), ProjectId(uuid(0xBAD))));
    assert!(matches!(got, Err(ProjectError::Forbidden)), "{got:?}");
}

/// The control for the matrix above: a workspace admin who is NOT in the project is refused by v1 although the internal API's
/// `updatable_project` lets them in, so the two rules are told apart.
#[test]
fn a_workspace_admin_outside_the_project_is_refused_by_v1_and_let_in_by_the_internal_api() {
    let w = world();
    let admin_outside = wscope(&w, Actor::OutsiderA, Role::Admin);
    assert!(matches!(block_on(project_writer(&w.app, &admin_outside, w.project_a)), Err(ProjectError::Forbidden)));
    assert!(block_on(updatable_project(&w.app, &admin_outside, w.project_a)).is_ok());
}

// --- changing -----------------------------------------------------------------------------------------------------------

/// A default state is one of the project's own: another project's is refused, so is one that is nowhere, and null clears it. An
/// archived project is refused first, whatever the patch says.
#[test]
fn a_change_names_a_default_state_of_the_project_and_an_archived_project_is_not_changed() {
    let w = seeded();
    let admin = real_scope(&w, Actor::AdminA);
    let set = |state: Option<Option<Uuid>>| {
        block_on(change_project(
            &w.app,
            &admin,
            w.project_a,
            ProjectPatch { default_state: state, ..ProjectPatch::default() },
        ))
    };

    assert_eq!(set(Some(Some(uuid(STATE_TODO)))).unwrap().listing.project.default_state, Some(uuid(STATE_TODO)));
    for not_ours in [uuid(STATE_OF_B), uuid(0xDEAD)] {
        let got = set(Some(Some(not_ours)));
        assert!(matches!(got, Err(ProjectError::NotAState)), "{not_ours}: {got:?}");
    }
    // a refusal changes nothing
    let kept = block_on(open_project(&w.app, &admin, w.project_a)).unwrap().listing.project.default_state;
    assert_eq!(kept, Some(uuid(STATE_TODO)));
    assert_eq!(set(Some(None)).unwrap().listing.project.default_state, None);
    // naming nothing leaves it as it is
    assert_eq!(set(None).unwrap().listing.project.default_state, None);

    block_on(set_project_archived(&w.app, &admin, w.project_a, true)).unwrap();
    let got = set(Some(Some(uuid(STATE_OF_B))));
    assert!(
        matches!(got, Err(ProjectError::Archived)),
        "an archived project is refused before its patch is read: {got:?}"
    );
}

/// The change is saved whole, with who made it and when, as the internal API's is.
#[test]
fn a_change_records_who_changed_the_project_and_when() {
    let w = world();
    let admin = real_scope(&w, Actor::AdminA);
    let d = block_on(change_project(
        &w.app,
        &admin,
        w.project_a,
        ProjectPatch { description: Some("v1".into()), ..ProjectPatch::default() },
    ))
    .unwrap();
    assert_eq!(d.listing.project.description, "v1");
    assert_eq!(d.listing.project.updated_by, Some(admin.user_id()));
    assert_eq!(d.listing.project.updated_at, w.app.ports.clock.now());
}

// --- archiving ----------------------------------------------------------------------------------------------------------

/// Archiving sets `archived_at` to now and records who; unarchiving clears it and records who; neither is for a caller who is not a
/// writer, and what is refused changes nothing.
#[test]
fn archiving_records_when_and_by_whom_and_unarchiving_clears_it() {
    let w = world();
    let admin = real_scope(&w, Actor::AdminA);
    let read = || block_on(open_project(&w.app, &admin, w.project_a));

    // a member of the project who is not a writer cannot
    let member = real_scope(&w, Actor::MemberA);
    assert!(matches!(block_on(set_project_archived(&w.app, &member, w.project_a, true)), Err(ProjectError::Forbidden)));
    assert!(read().unwrap().listing.project.archived_at.is_none());

    block_on(set_project_archived(&w.app, &admin, w.project_a, true)).unwrap();
    // `open_project` is the internal API's and does not show an archived project; the row is read from the port
    let row = block_on(w.app.ports.db.project_detail(&admin, w.project_a)).unwrap().unwrap().listing.project;
    assert_eq!(row.archived_at, Some(w.app.ports.clock.now()));
    assert_eq!(row.updated_by, Some(admin.user_id()));
    assert!(matches!(read(), Err(ProjectError::NotFound)));

    // archiving an archived project is archiving it again
    block_on(set_project_archived(&w.app, &admin, w.project_a, true)).unwrap();

    block_on(set_project_archived(&w.app, &admin, w.project_a, false)).unwrap();
    assert!(read().unwrap().listing.project.archived_at.is_none());
}

// --- deleting -----------------------------------------------------------------------------------------------------------

/// Only a writer deletes, and what is refused deletes nothing.
#[rstest]
fn only_a_writer_deletes_a_project(
    #[values(Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA)] who: Actor,
    #[values(Role::Member, Role::Admin)] ws_role: Role,
) {
    let w = world();
    let got = block_on(delete_project(&w.app, &wscope(&w, who, ws_role), w.project_a));
    let project = project_role(who);
    let allowed = project == Some(Role::Admin) || (project.is_some() && ws_role == Role::Admin);
    let still_there = block_on(w.app.ports.db.project_detail(&real_scope(&w, Actor::AdminA), w.project_a)).unwrap();
    match (allowed, &got) {
        (true, Ok(())) => assert!(still_there.is_none(), "{who:?} as {ws_role:?}"),
        (false, Err(ProjectError::Forbidden)) => assert!(still_there.is_some(), "{who:?} as {ws_role:?}"),
        _ => panic!("{who:?} as {ws_role:?}: {got:?}"),
    }
}

/// A deleted project is gone from every read the port has for a project, its name and key are free, and a second deletion is refused
/// (there is no membership left to find). Every read is asked, none is skipped: a new one must be added here.
#[test]
fn a_deleted_project_leaves_every_read_and_frees_its_name_and_key() {
    let w = seeded();
    let admin = real_scope(&w, Actor::AdminA);
    let before = block_on(open_project(&w.app, &admin, w.project_a)).unwrap().listing.project;
    let db = &w.app.ports.db;

    block_on(delete_project(&w.app, &admin, w.project_a)).unwrap();

    // by id
    assert!(block_on(db.project_detail(&admin, w.project_a)).unwrap().is_none());
    assert!(matches!(block_on(open_project(&w.app, &admin, w.project_a)), Err(ProjectError::NotFound)));
    assert!(block_on(db.project_member(&admin, w.project_a)).unwrap().is_none());
    assert!(!block_on(db.update_project(&admin, &before)).unwrap(), "a deleted project is not saved");
    // by the caller's standing: no membership means nothing under the project can be reached either
    let got = block_on(authorize(&w.app, &w.principal(Actor::AdminA), &w.slug_a, w.project_a, Permission::ReadProject));
    assert!(matches!(got, Err(CoreError::NotFound)), "{got:?}");
    // in the lists
    let ids = |l: &[ProjectListing]| -> Vec<ProjectId> { l.iter().map(|x| x.project.id).collect() };
    assert!(!ids(&block_on(db.workspace_projects(&admin)).unwrap()).contains(&w.project_a));
    assert!(!ids(&block_on(db.visible_projects(&admin)).unwrap()).contains(&w.project_a));
    assert!(!block_on(db.project_roles(&admin)).unwrap().iter().any(|(p, _)| *p == w.project_a));
    assert!(!block_on(db.project_stats(&admin, None)).unwrap().iter().any(|s| s.id == w.project_a));
    assert!(block_on(db.workspace_states(&admin)).unwrap().iter().all(|s| s.project_id != w.project_a));
    // its name and key are free, and a project can be made with them
    let c = block_on(project_clashes(&w.app, &admin, Some(&before.name), Some(&before.identifier), None)).unwrap();
    assert_eq!((c.name, c.identifier), (false, false));
    let again = block_on(create_project(&w.app, &admin, patch(&before.name, &before.identifier))).unwrap();
    assert_ne!(again.listing.project.id, w.project_a);

    // and a second deletion finds no one to refuse or allow
    assert!(matches!(block_on(delete_project(&w.app, &admin, w.project_a)), Err(ProjectError::Forbidden)));
}

// --- who sees one project -----------------------------------------------------------------------------------------------

/// One project as v1's queryset finds it: the caller is a member of it or it is public, whatever the caller's role in the workspace,
/// and an archived one is found as well. The matrix is every caller (a member of project A or not) against a public project, a secret
/// one, an archived one and one that is not there; project A is public, and `OutsiderA` makes the secret one and is its only member.
#[test]
fn a_project_is_found_by_its_members_and_by_anyone_if_it_is_public() {
    let w = world();
    let secret = block_on(create_project(
        &w.app,
        &real_scope(&w, Actor::OutsiderA),
        ProjectPatch { network: Some(ProjectNetwork::Secret), ..patch("Secret S", "SEC") },
    ))
    .unwrap()
    .listing
    .project
    .id;
    // an archived public project: the creator archives it
    let archived = block_on(create_project(&w.app, &real_scope(&w, Actor::MemberA), patch("Old", "OLD")))
        .unwrap()
        .listing
        .project
        .id;
    block_on(set_project_archived(&w.app, &real_scope(&w, Actor::MemberA), archived, true)).unwrap();
    let nowhere = ProjectId(uuid(0xBAD));

    for who in [Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA] {
        let scope = real_scope(&w, who);
        let found = |id: ProjectId| block_on(visible_project(&w.app, &scope, id));
        // a public project is found by everyone (A: three are in it, the fourth is not), and an archived one too
        assert!(found(w.project_a).is_ok(), "{who:?} finds a public project");
        let old = found(archived).unwrap_or_else(|e| panic!("{who:?} finds an archived public project: {e:?}"));
        assert!(old.listing.project.archived_at.is_some());
        // a secret one only by its member
        let got = found(secret);
        if who == Actor::OutsiderA {
            assert_eq!(got.unwrap().listing.role, Some(Role::Admin));
        } else {
            assert!(matches!(got, Err(ProjectError::NotFound)), "{who:?}: {got:?}");
        }
        // one that is not there is not found
        assert!(matches!(found(nowhere), Err(ProjectError::NotFound)));
    }
    // being in a secret project is what finds it: the same caller, a member of it, is found by a project they are not in only if public
    let member_of_a_public = real_scope(&w, Actor::GuestA);
    assert_eq!(
        block_on(visible_project(&w.app, &member_of_a_public, w.project_a)).unwrap().listing.role,
        Some(Role::Guest)
    );
    assert_eq!(block_on(visible_project(&w.app, &member_of_a_public, archived)).unwrap().listing.role, None);
}

// --- who sees which projects --------------------------------------------------------------------------------------------

/// v1's rule, whatever the caller's role in the workspace: the projects they are in, and the public ones. Project A is public
/// (three members), S is secret and only its creator (who is in the workspace and not in A) is in it, P is public. `world()` makes
/// every workspace member a plain member, so the test gives `AdminA` the workspace's admin role and `GuestA` its guest role: then the
/// workspace admin does not see S and the workspace guest does see P, and the internal API's `workspace_projects` says the opposite
/// of both and is asked beside it.
#[test]
fn a_caller_sees_the_projects_they_are_in_and_the_public_ones_whatever_their_role() {
    let w = world();
    let role =
        |who: Actor, role: Role| SeedWorkspaceRole { workspace_id: w.ws_a, user_id: w.principal(who).user_id, role };
    let roles = SeedFixture {
        workspace_roles: vec![role(Actor::AdminA, Role::Admin), role(Actor::GuestA, Role::Guest)],
        ..SeedFixture::default()
    };
    block_on(w.app.ports.db.seed(&SystemScope::mint(), &roles)).unwrap();
    assert_eq!(real_scope(&w, Actor::AdminA).role(), Role::Admin, "the premise: a workspace admin");
    assert_eq!(real_scope(&w, Actor::GuestA).role(), Role::Guest, "the premise: a workspace guest");
    let outsider = real_scope(&w, Actor::OutsiderA);
    let secret = block_on(create_project(
        &w.app,
        &outsider,
        ProjectPatch { network: Some(ProjectNetwork::Secret), ..patch("Secret S", "SEC") },
    ))
    .unwrap()
    .listing
    .project
    .id;
    let public = block_on(create_project(&w.app, &real_scope(&w, Actor::MemberA), patch("Public P", "PUB")))
        .unwrap()
        .listing
        .project
        .id;

    let sees = |who: Actor| -> BTreeSet<ProjectId> {
        block_on(visible_projects(&w.app, &real_scope(&w, who))).unwrap().iter().map(|l| l.project.id).collect()
    };
    let set = |ids: &[ProjectId]| ids.iter().copied().collect::<BTreeSet<_>>();
    // the workspace's admin, a member and a guest of A: A and P (public), never S
    for who in [Actor::AdminA, Actor::MemberA, Actor::GuestA] {
        assert_eq!(sees(who), set(&[w.project_a, public]), "{who:?}");
    }
    // the creator of S is in it, and sees it besides
    assert_eq!(sees(Actor::OutsiderA), set(&[w.project_a, secret, public]));

    // the internal API's rule differs where the two were said to: the admin sees S there, the guest does not see P
    let internal = |who: Actor| -> BTreeSet<ProjectId> {
        block_on(w.app.ports.db.workspace_projects(&real_scope(&w, who)))
            .unwrap()
            .iter()
            .map(|l| l.project.id)
            .collect()
    };
    assert!(internal(Actor::AdminA).contains(&secret));
    assert!(!internal(Actor::GuestA).contains(&public));

    // the caller's own role in each, and the list is oldest first (ties by id)
    let listing = block_on(visible_projects(&w.app, &real_scope(&w, Actor::MemberA))).unwrap();
    for l in &listing {
        let expected = if l.project.id == w.project_a {
            Some(Role::Member)
        } else if l.project.id == public {
            Some(Role::Admin)
        } else {
            None
        };
        assert_eq!(l.role, expected, "{}", l.project.name);
    }
    let order: Vec<_> = listing.iter().map(|l| (l.project.created_at, l.project.id)).collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted);
}
