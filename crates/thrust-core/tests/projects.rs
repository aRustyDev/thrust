//! Projects (issue #44, the project stage): making one, changing it, opening it, and what the app reads about one, against the
//! memory adapters.
//!
//! What must hold: a member or admin of the workspace makes a project and is its admin, with the pin's states, and so is the lead
//! they name; a name and a key follow the pin's rules (a length, no forbidden character, taken or not) and a refused creation
//! writes nothing; the workspace's admins and a project's own admins change it, nobody else, and not an archived one; a project is
//! opened by its members and refused to everyone else in the three ways the pin tells apart.

mod common;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

fn wscope(w: &World, who: Actor, role: Role) -> WorkspaceScope {
    WorkspaceScope::mint(w.ws_a, w.principal(who).user_id, role)
}

fn patch(name: &str, identifier: &str) -> ProjectPatch {
    ProjectPatch { name: Some(name.into()), identifier: Some(identifier.into()), ..ProjectPatch::default() }
}

fn made(w: &World, who: Actor, role: Role, p: ProjectPatch) -> Result<ProjectDetail, ProjectError> {
    block_on(create_project(&w.app, &wscope(w, who, role), p))
}

// --- making a project --------------------------------------------------------------------------------------------------------

/// Spec: Plane makes a project for an admin or a member of the workspace, and not for a guest.
#[rstest]
fn only_an_admin_or_a_member_of_the_workspace_makes_a_project(#[values(Role::Guest, Role::Member, Role::Admin)] role: Role) {
    let w = world();
    let got = made(&w, Actor::OutsiderA, role, patch("New", "NEW"));
    match role {
        Role::Guest => assert!(matches!(got, Err(ProjectError::Forbidden))),
        _ => assert!(got.is_ok(), "{got:?}"),
    }
    // a refusal writes nothing
    let clashes = block_on(project_clashes(&w.app, &wscope(&w, Actor::OutsiderA, Role::Admin), Some("New"), Some("NEW"), None)).unwrap();
    assert_eq!((clashes.name, clashes.identifier), (role != Role::Guest, role != Role::Guest));
}

/// Spec (the pin's `ProjectViewSet.create` and `DEFAULT_STATES`): the creator is the project's admin, and so is a lead who is someone
/// else (listed first: their row is the newer); the project has the five states and the intake one, with the pin's colours and
/// places, the first of the five the default; and each person has their settings row with the pin's defaults.
#[test]
fn a_new_project_has_its_creator_and_lead_as_admins_the_pins_states_and_each_persons_settings() {
    let w = world();
    let lead = w.principal(Actor::MemberA).user_id;
    let creator = w.principal(Actor::OutsiderA).user_id;
    let d = made(&w, Actor::OutsiderA, Role::Member, ProjectPatch { project_lead: Some(Some(lead)), ..patch("New", "NEW") }).unwrap();
    assert_eq!((d.listing.role, d.listing.sort_order), (Some(Role::Admin), Some(65535.0)));
    assert_eq!(d.members, vec![lead, creator], "the lead's row is the newer");
    assert_eq!((d.listing.project.created_by, d.listing.project.project_lead, d.next_sequence), (Some(creator), Some(lead), 1));
    let id = d.listing.project.id;
    let scope = block_on(authorize(&w.app, &w.principal(Actor::OutsiderA), &w.slug_a, id, Permission::ReadProject)).unwrap();

    let states: Vec<(String, String, f64, StateGroup, bool)> =
        block_on(project_states(&w.app, &scope)).unwrap().into_iter().map(|s| (s.name, s.color, s.sequence, s.group, s.is_default)).collect();
    let spec = [
        ("Backlog", "#60646C", 15000.0, StateGroup::Backlog, true),
        ("Todo", "#60646C", 25000.0, StateGroup::Unstarted, false),
        ("In Progress", "#F59E0B", 35000.0, StateGroup::Started, false),
        ("Done", "#46A758", 45000.0, StateGroup::Completed, false),
        ("Cancelled", "#9AA4BC", 55000.0, StateGroup::Cancelled, false),
    ];
    assert_eq!(states, spec.map(|(n, c, s, g, d)| (n.to_string(), c.to_string(), s, g, d)));
    let triage = block_on(intake_state(&w.app, &scope)).unwrap();
    assert_eq!((triage.name.as_str(), triage.color.as_str(), triage.sequence, triage.group), ("Triage", "#4E5355", 65000.0, StateGroup::Triage));

    // the creator's settings are there from the start, with the pin's defaults; the lead's are theirs
    let props = block_on(project_properties(&w.app, &scope)).unwrap();
    assert_eq!((props.sort_order, props.rich_filters.clone()), (65535.0, serde_json::json!({})));
    assert_eq!(props.display_filters["layout"], "list");
    assert_eq!(props.preferences["navigation"]["default_tab"], "work_items");
    let members = block_on(project_members(&w.app, &scope)).unwrap();
    assert_eq!(members.iter().map(|m| (m.user_id, m.role)).collect::<Vec<_>>(), vec![(lead, Role::Admin), (creator, Role::Admin)]);
}

/// The lead who is the creator is one member, not two.
#[test]
fn a_lead_who_is_the_creator_is_one_member() {
    let w = world();
    let me = w.principal(Actor::OutsiderA).user_id;
    let d = made(&w, Actor::OutsiderA, Role::Member, ProjectPatch { project_lead: Some(Some(me)), ..patch("New", "NEW") }).unwrap();
    assert_eq!(d.members, vec![me]);
}

#[test]
fn a_name_and_a_key_that_are_taken_are_a_conflict_and_a_project_in_another_workspace_does_not_count() {
    let w = world();
    made(&w, Actor::OutsiderA, Role::Member, patch("New", "NEW")).unwrap();
    for (name, identifier) in [("New", "OTHER"), ("Other", "NEW")] {
        let got = made(&w, Actor::OutsiderA, Role::Member, patch(name, identifier));
        assert!(matches!(got, Err(ProjectError::Core(CoreError::Conflict(_)))), "{name} {identifier}: {got:?}");
    }
    // exact: another case is another name
    assert!(made(&w, Actor::OutsiderA, Role::Member, patch("new", "new")).is_ok());
    // the same name and key in workspace B
    let b = WorkspaceScope::mint(w.ws_b, w.principal(Actor::MemberB).user_id, Role::Member);
    assert!(block_on(create_project(&w.app, &b, patch("New", "NEW"))).is_ok());
}

/// The pin's `FORBIDDEN_IDENTIFIER_CHARS_PATTERN` characters, each one, in a name and in a key.
#[rstest]
fn each_forbidden_character_refuses_a_name_and_a_key(
    #[values('&', '+', ',', ':', ';', '$', '^', '}', '{', '*', '=', '?', '@', '#', '|', '\'', '<', '>', '.', '(', ')', '%', '!', '-')] c: char,
    #[values(true, false)] in_name: bool,
) {
    let w = world();
    let (name, key) = if in_name { (format!("a{c}b"), "OK".to_string()) } else { ("Fine".to_string(), format!("A{c}B")) };
    let got = made(&w, Actor::OutsiderA, Role::Member, patch(&name, &key));
    assert!(matches!(got, Err(ProjectError::Core(CoreError::Invalid(_)))), "{c:?} in {}: {got:?}", if in_name { "the name" } else { "the key" });
}

proptest! {
    #![proptest_config(proptest_config(512))]

    /// Differential against the pin's own pattern, `re.match(r"^.*[&+,:;$^}{*=?@#|'<>.()%!-].*$", s)`, which Python's `re` reads as: a
    /// forbidden character on the first line and no other line, a single trailing newline aside. The regex crate reads
    /// `^[^\n]*[...][^\n]*\n?$` the same way. The alphabet holds newlines so that the quirk (a forbidden character after a newline in
    /// the middle is not found) is reached.
    #[test]
    fn prop_forbidden_characters_are_found_as_the_pins_pattern_finds_them(s in "[a-c&.\\-\n \u{e9}]{0,10}") {
        let oracle = regex::Regex::new(r"^[^\n]*[&+,:;$^}{*=?@#|'<>.()%!\-][^\n]*\n?$").unwrap();
        prop_assert_eq!(has_forbidden_project_chars(&s), oracle.is_match(&s), "{:?}", s);
    }
}

/// Pinned, from the pin's pattern: a forbidden character after a line break is not found (the quirk the property above reaches).
#[rstest]
#[case("a-b", true)]
#[case("a-b\n", true)]
#[case("a\n-b", false)]
#[case("a-\nb", false)]
#[case("ab\n", false)]
fn the_pins_pattern_reads_one_line(#[case] s: &str, #[case] found: bool) {
    assert_eq!(has_forbidden_project_chars(s), found, "{s:?}");
}

/// Spec: a name is 1 to 255 characters and a key 1 to 12 (counted in characters, not bytes), a time zone exists, months run from 0
/// to 12, and a lead is a user.
#[rstest]
#[case::name_255(&"é".repeat(255), "OK", true)]
#[case::name_256(&"é".repeat(256), "OK", false)]
#[case::name_empty("", "OK", false)]
#[case::key_12("Fine", &"é".repeat(12), true)]
#[case::key_13("Fine", &"é".repeat(13), false)]
#[case::key_empty("Fine", "", false)]
fn a_name_and_a_key_have_the_pins_lengths(#[case] name: &str, #[case] key: &str, #[case] ok: bool) {
    let w = world();
    let got = made(&w, Actor::OutsiderA, Role::Member, patch(name, key));
    assert_eq!(got.is_ok(), ok, "{got:?}");
    if !ok {
        assert!(matches!(got, Err(ProjectError::Core(CoreError::Invalid(_)))));
    }
}

#[rstest]
#[case::a_time_zone_that_does_not_exist(ProjectPatch { timezone: Some("Mars/Olympus".into()), ..patch("New", "NEW") }, false)]
#[case::a_time_zone_that_does(ProjectPatch { timezone: Some("Europe/Berlin".into()), ..patch("New", "NEW") }, true)]
#[case::months_13(ProjectPatch { archive_in: Some(13), ..patch("New", "NEW") }, false)]
#[case::months_negative(ProjectPatch { close_in: Some(-1), ..patch("New", "NEW") }, false)]
#[case::months_12(ProjectPatch { archive_in: Some(12), close_in: Some(0), ..patch("New", "NEW") }, true)]
#[case::no_name(ProjectPatch { name: None, ..patch("New", "NEW") }, false)]
#[case::no_key(ProjectPatch { identifier: None, ..patch("New", "NEW") }, false)]
fn a_project_must_satisfy_the_pins_other_rules(#[case] p: ProjectPatch, #[case] ok: bool) {
    let w = world();
    let got = made(&w, Actor::OutsiderA, Role::Member, p);
    assert_eq!(got.is_ok(), ok, "{got:?}");
    if !ok {
        assert!(matches!(got, Err(ProjectError::Core(CoreError::Invalid(_)))), "{got:?}");
    }
}

#[test]
fn a_lead_must_be_a_user() {
    let w = world();
    let nobody = UserId(Uuid::from_u128(0xDEAD));
    let got = made(&w, Actor::OutsiderA, Role::Member, ProjectPatch { project_lead: Some(Some(nobody)), ..patch("New", "NEW") });
    assert!(matches!(got, Err(ProjectError::Core(CoreError::Invalid(_)))), "{got:?}");
    let got = made(&w, Actor::OutsiderA, Role::Member, ProjectPatch { default_assignee: Some(Some(nobody)), ..patch("New", "NEW") });
    assert!(matches!(got, Err(ProjectError::Core(CoreError::Invalid(_)))), "{got:?}");
    assert!(block_on(project_clashes(&w.app, &wscope(&w, Actor::OutsiderA, Role::Admin), Some("New"), None, None)).unwrap().name == false);
}

/// Deviation D-026: the pin lets any user lead a project, Thrust's project members are workspace members, so a lead who is not an
/// active member of the project's workspace is refused (`Invalid`) and nothing is written. Every actor of the world is named lead
/// once: the members of workspace A are accepted, and so is the creator; the member of workspace B only is not.
#[rstest]
fn a_lead_must_be_a_member_of_the_workspace(#[values(Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA, Actor::MemberB, Actor::Dual)] lead: Actor) {
    let w = world();
    let got = made(&w, Actor::OutsiderA, Role::Member, ProjectPatch { project_lead: Some(Some(w.principal(lead).user_id)), ..patch("New", "NEW") });
    if lead == Actor::MemberB {
        assert!(matches!(got, Err(ProjectError::Core(CoreError::Invalid(_)))), "{got:?}");
        let admin = wscope(&w, Actor::AdminA, Role::Admin);
        let clashes = block_on(project_clashes(&w.app, &admin, Some("New"), Some("NEW"), None)).unwrap();
        assert_eq!((clashes.name, clashes.identifier), (false, false), "a refused creation writes nothing");
    } else {
        assert!(got.is_ok(), "{lead:?}: {got:?}");
    }
}

/// D-026 again, for a member who has left the workspace (deactivated): the row exists, and still the lead may not be them.
#[test]
fn a_lead_who_has_left_the_workspace_is_refused() {
    let w = world();
    let gone = w.principal(Actor::MemberA).user_id;
    assert!(block_on(w.app.ports.db.set_workspace_member_active(&SystemScope::mint(), w.ws_a, gone, false)).unwrap());
    let got = made(&w, Actor::OutsiderA, Role::Member, ProjectPatch { project_lead: Some(Some(gone)), ..patch("New", "NEW") });
    assert!(matches!(got, Err(ProjectError::Core(CoreError::Invalid(_)))), "{got:?}");
}

/// D-026 for a change: naming a lead who is not a member is refused and the project keeps its lead; a request that names no lead is
/// not asked (the lead the project has stays whatever it is).
#[test]
fn a_change_that_names_a_lead_outside_the_workspace_is_refused_and_one_that_names_none_is_not_asked() {
    let w = world();
    let admin = wscope(&w, Actor::AdminA, Role::Admin);
    let stranger = w.principal(Actor::MemberB).user_id;
    let got = block_on(update_project(&w.app, &admin, w.project_a, ProjectPatch { project_lead: Some(Some(stranger)), ..ProjectPatch::default() }));
    assert!(matches!(got, Err(ProjectError::Core(CoreError::Invalid(_)))), "{got:?}");
    let kept = block_on(open_project(&w.app, &admin, w.project_a)).unwrap();
    assert_eq!(kept.listing.project.project_lead, None);
    let member = w.principal(Actor::MemberA).user_id;
    let got = block_on(update_project(&w.app, &admin, w.project_a, ProjectPatch { project_lead: Some(Some(member)), ..ProjectPatch::default() })).unwrap();
    assert_eq!(got.listing.project.project_lead, Some(member));
    let got = block_on(update_project(&w.app, &admin, w.project_a, ProjectPatch { description: Some("x".into()), ..ProjectPatch::default() })).unwrap();
    assert_eq!(got.listing.project.project_lead, Some(member));
    // the lead leaves the workspace: a change that names no lead still goes through, and the project keeps its lead
    assert!(block_on(w.app.ports.db.set_workspace_member_active(&SystemScope::mint(), w.ws_a, member, false)).unwrap());
    let got = block_on(update_project(&w.app, &admin, w.project_a, ProjectPatch { description: Some("y".into()), ..ProjectPatch::default() })).unwrap();
    assert_eq!((got.listing.project.description.as_str(), got.listing.project.project_lead), ("y", Some(member)));
}

/// `is_workspace_member` says of anyone (the caller or not) whether they are an active member of the scope's workspace: the
/// members of workspace A in every role, and not the member of workspace B only, nor a member who has left.
#[rstest]
fn anyone_is_asked_whether_they_are_an_active_member_of_the_workspace(
    #[values(Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA, Actor::MemberB, Actor::Dual)] who: Actor,
    #[values(Actor::AdminA, Actor::GuestA)] asking: Actor,
    #[values(true, false)] left: bool,
) {
    let w = world();
    let user = w.principal(who).user_id;
    if left {
        block_on(w.app.ports.db.set_workspace_member_active(&SystemScope::mint(), w.ws_a, user, false)).unwrap();
    }
    let got = block_on(is_workspace_member(&w.app, &wscope(&w, asking, Role::Admin), user)).unwrap();
    assert_eq!(got, who != Actor::MemberB && !left, "{who:?} asked by {asking:?}, left: {left}");
}

/// A project's logo is an object (the column holds one, which the pin does not require): the core refuses anything else, whoever
/// asks, on a creation and on a change.
#[rstest]
fn a_logo_that_is_not_an_object_is_invalid(#[values(serde_json::json!("x"), serde_json::json!([1]), serde_json::json!(5), serde_json::json!(null))] logo: serde_json::Value) {
    let w = world();
    let got = made(&w, Actor::OutsiderA, Role::Member, ProjectPatch { logo_props: Some(logo.clone()), ..patch("New", "NEW") });
    assert!(matches!(got, Err(ProjectError::Core(CoreError::Invalid(_)))), "{logo}: {got:?}");
    let admin = wscope(&w, Actor::AdminA, Role::Admin);
    let got = block_on(update_project(&w.app, &admin, w.project_a, ProjectPatch { logo_props: Some(logo.clone()), ..ProjectPatch::default() }));
    assert!(matches!(got, Err(ProjectError::Core(CoreError::Invalid(_)))), "{logo}: {got:?}");
    let ok = made(&w, Actor::OutsiderA, Role::Member, ProjectPatch { logo_props: Some(serde_json::json!({"in_use": "emoji"})), ..patch("Fine", "FIN") });
    assert_eq!(ok.unwrap().listing.project.logo_props, serde_json::json!({"in_use": "emoji"}));
}

/// The states of the workspace are those of the projects the caller is in (not of every project): a project's creator lists its
/// five, a member of the workspace who is not in it none of them.
#[test]
fn the_workspaces_states_are_those_of_the_projects_the_caller_is_in() {
    let w = world();
    let d = made(&w, Actor::OutsiderA, Role::Member, patch("New", "NEW")).unwrap();
    let id = d.listing.project.id;
    let of = |who: Actor| -> Vec<String> {
        block_on(workspace_states(&w.app, &wscope(&w, who, Role::Member))).unwrap().into_iter().filter(|s| s.project_id == id).map(|s| s.name).collect()
    };
    assert_eq!(of(Actor::OutsiderA), ["Backlog", "Todo", "In Progress", "Done", "Cancelled"]);
    assert!(of(Actor::MemberA).is_empty());
}

/// A project's labels are listed by `sort_order` (the core only reads them: nothing makes one yet).
#[test]
fn a_projects_labels_are_listed_by_sort_order() {
    let w = world();
    let label = |n: u128, name: &str, sort_order: f64| SeedLabel {
        id: Uuid::from_u128(n),
        project_id: w.project_a,
        workspace_id: w.ws_a,
        name: name.into(),
        color: "#fff".into(),
        parent_id: None,
        sort_order,
    };
    block_on(w.app.ports.db.seed(&SystemScope::mint(), &SeedFixture { labels: vec![label(1, "later", 20.0), label(2, "first", 10.0)], ..SeedFixture::default() })).unwrap();
    let scope = block_on(authorize(&w.app, &w.principal(Actor::AdminA), &w.slug_a, w.project_a, Permission::ReadProject)).unwrap();
    let names: Vec<String> = block_on(project_labels(&w.app, &scope)).unwrap().into_iter().map(|l| l.name).collect();
    assert_eq!(names, ["first", "later"]);
}

// --- changing a project ------------------------------------------------------------------------------------------------------

/// Spec (the pin's `partial_update`): the workspace's admins and the project's own admins may change a project; nobody else in the
/// workspace may, a member or guest of the project included.
#[rstest]
fn who_may_change_a_project(
    #[values(Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA)] who: Actor,
    #[values(Role::Guest, Role::Member, Role::Admin)] role: Role,
) {
    let w = world();
    let project_admin = who == Actor::AdminA;
    let allowed = role == Role::Admin || project_admin;
    let got = block_on(update_project(&w.app, &wscope(&w, who, role), w.project_a, ProjectPatch { description: Some("changed".into()), ..ProjectPatch::default() }));
    match (allowed, &got) {
        (true, Ok(d)) => assert_eq!(d.listing.project.description, "changed"),
        (false, Err(ProjectError::Forbidden)) => {}
        _ => panic!("{who:?} as {role:?}: {got:?}"),
    }
    // and what is not allowed changes nothing
    let after = block_on(open_project(&w.app, &wscope(&w, Actor::AdminA, Role::Member), w.project_a)).unwrap();
    assert_eq!(after.listing.project.description == "changed", allowed);
}

/// A project that is not there is a 404 to a workspace admin and a refusal to everyone else, as the pin checks the role first.
#[rstest]
fn a_project_that_is_not_there_cannot_be_changed(#[values(Role::Guest, Role::Member, Role::Admin)] role: Role) {
    let w = world();
    let got = block_on(update_project(&w.app, &wscope(&w, Actor::AdminA, role), ProjectId(Uuid::from_u128(0xBAD)), ProjectPatch::default()));
    match role {
        Role::Admin => assert!(matches!(got, Err(ProjectError::NotFound)), "{got:?}"),
        _ => assert!(matches!(got, Err(ProjectError::Forbidden)), "{got:?}"),
    }
}

/// A change is a whole save: it sets what the patch names, who changed it and when, keeps who made it and when, refuses a project
/// that is archived (whatever the patch says), and a name that another project has.
#[test]
fn a_change_sets_what_it_names_saves_who_and_when_and_refuses_an_archived_project_or_a_taken_name() {
    let w = world();
    let admin = wscope(&w, Actor::AdminA, Role::Admin);
    let before = block_on(open_project(&w.app, &admin, w.project_a)).unwrap().listing.project;
    let p = ProjectPatch { name: Some("Renamed".into()), network: Some(ProjectNetwork::Secret), page_view: Some(false), emoji: Some(Some("🙂".into())), ..ProjectPatch::default() };
    let d = block_on(update_project(&w.app, &admin, w.project_a, p)).unwrap().listing.project;
    assert_eq!((d.name.as_str(), d.network, d.page_view, d.emoji.as_deref()), ("Renamed", ProjectNetwork::Secret, false, Some("🙂")));
    assert_eq!((d.updated_by, d.updated_at), (Some(w.principal(Actor::AdminA).user_id), w.app.ports.clock.now()));
    assert_eq!((d.created_by, d.created_at, d.identifier.as_str()), (before.created_by, before.created_at, before.identifier.as_str()));
    // an empty patch still saves: the time and who changed it move (the pin's `save()` writes every column)
    assert!(block_on(update_project(&w.app, &admin, w.project_a, ProjectPatch::default())).is_ok());
    // the name of another project of the workspace
    made(&w, Actor::OutsiderA, Role::Member, patch("Other", "OTH")).unwrap();
    let got = block_on(update_project(&w.app, &admin, w.project_a, ProjectPatch { name: Some("Other".into()), ..ProjectPatch::default() }));
    assert!(matches!(got, Err(ProjectError::Core(CoreError::Conflict(_)))), "{got:?}");
    // archived
    let mut row = block_on(open_project(&w.app, &admin, w.project_a)).unwrap().listing.project;
    row.archived_at = Some(w.app.ports.clock.now());
    assert!(block_on(w.app.ports.db.update_project(&admin, &row)).unwrap());
    for patch in [ProjectPatch::default(), ProjectPatch { name: Some("x".into()), ..ProjectPatch::default() }, ProjectPatch { name: Some("bad.name".into()), ..ProjectPatch::default() }] {
        let got = block_on(update_project(&w.app, &admin, w.project_a, patch));
        assert!(matches!(got, Err(ProjectError::Archived)), "{got:?}");
    }
}

// --- opening a project -------------------------------------------------------------------------------------------------------

/// Spec (the pin's `retrieve`): a member of the project opens it; anyone else in the workspace is told it is secret (403) or that
/// they are not a member (409), whatever their role in the workspace; a project that is archived or not there is a 404 for all.
#[rstest]
fn a_project_is_opened_by_its_members_and_refused_to_everyone_else_in_three_ways(
    #[values(Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA)] who: Actor,
    #[values(Role::Guest, Role::Member, Role::Admin)] role: Role,
    #[values(ProjectNetwork::Public, ProjectNetwork::Secret)] network: ProjectNetwork,
    #[values(false, true)] archived: bool,
) {
    let w = world();
    let admin = wscope(&w, Actor::AdminA, Role::Admin);
    let mut row = block_on(open_project(&w.app, &admin, w.project_a)).unwrap().listing.project;
    row.network = network;
    row.archived_at = archived.then(|| w.app.ports.clock.now());
    assert!(block_on(w.app.ports.db.update_project(&admin, &row)).unwrap());
    let got = block_on(open_project(&w.app, &wscope(&w, who, role), w.project_a));
    let member = who != Actor::OutsiderA;
    match (archived, member, network, &got) {
        (true, ..) => assert!(matches!(got, Err(ProjectError::NotFound)), "{got:?}"),
        (false, true, _, Ok(_)) => {}
        (false, false, ProjectNetwork::Secret, Err(ProjectError::Secret)) => {}
        (false, false, ProjectNetwork::Public, Err(ProjectError::NotAMember)) => {}
        _ => panic!("{who:?} as {role:?}, {network:?}, archived {archived}: {got:?}"),
    }
}

#[test]
fn a_project_that_is_not_there_or_is_of_another_workspace_is_not_found() {
    let w = world();
    let admin = wscope(&w, Actor::AdminA, Role::Admin);
    assert!(matches!(block_on(open_project(&w.app, &admin, ProjectId(Uuid::from_u128(0xBAD)))), Err(ProjectError::NotFound)));
    assert!(matches!(block_on(open_project(&w.app, &admin, w.project_b)), Err(ProjectError::NotFound)));
}

// --- what the app reads about a project --------------------------------------------------------------------------------------

#[test]
fn the_workspaces_counts_and_the_callers_own_row_pass_through_a_scope() {
    let w = world();
    let admin = wscope(&w, Actor::AdminA, Role::Admin);
    let stats = block_on(project_stats(&w.app, &admin, None)).unwrap();
    assert_eq!(stats.iter().map(|s| (s.id, s.total_members)).collect::<Vec<_>>(), vec![(w.project_a, 4)]);
    let me = block_on(my_project_member(&w.app, &admin, w.project_a)).unwrap().unwrap();
    assert_eq!((me.member.role, me.project.id, me.workspace.id), (Role::Admin, w.project_a, w.ws_a));
    let nobody = wscope(&w, Actor::OutsiderA, Role::Member);
    assert!(block_on(my_project_member(&w.app, &nobody, w.project_a)).unwrap().is_none());
}
