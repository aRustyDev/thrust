//! What the app reads when a member opens a workspace (issue #44): the projects they can see and the rows their first visit makes
//! (the sidebar, the home page's widgets, the work-item list settings), against the memory adapters.
//!
//! What must hold: the defaults are the pin's (the order of its enums, its pins and its places; each is named in a comment as
//! plane's own source or as the fixture the pin was recorded in); a visit makes a member's rows once, whatever the clock says
//! on the next one, and one member's rows or one workspace's are never another's; and the project list reaches only through a
//! scope, so it holds the projects of THAT workspace and carries the caller's own role in each.

mod common;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::json;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

/// A clock that moves one second on every reading, so a row made on a later visit would differ in time from the first's.
struct TickClock(AtomicI64);
impl Clock for TickClock {
    fn now(&self) -> Timestamp {
        DateTime::from_timestamp(1_760_000_000 + self.0.fetch_add(1, Ordering::Relaxed), 0).unwrap()
    }
}

/// The default world with a ticking clock.
fn ticking() -> World {
    let mut w = world();
    w.app.ports.clock = Arc::new(TickClock(AtomicI64::new(0)));
    w
}

/// The caller's way into a workspace, made the way a request makes it.
fn scope_of(w: &World, who: Actor, proj: Proj) -> WorkspaceScope {
    block_on(authorize_workspace(&w.app, &w.principal(who), &w.slug(proj, Slug::Own))).expect("the actor is a member")
}

// --- the defaults ------------------------------------------------------------------------------------------------------------

/// Spec: the pin's `WorkspaceUserPreference.UserPreferenceKeys` in its order, the ones it pins (drafts, your work, stickies),
/// and `65535 + 10000 * position` (the fixture S21, recorded from the pin).
#[test]
fn a_new_members_sidebar_is_the_pins_seven_entries_with_its_pins_and_places() {
    let w = world();
    let rows = block_on(sidebar_preferences(&w.app, &scope_of(&w, Actor::AdminA, Proj::A))).unwrap();
    let got: Vec<(&str, bool, f64)> = rows.iter().map(|r| (r.key.as_str(), r.is_pinned, r.sort_order)).collect();
    assert_eq!(
        got,
        [
            ("views", false, 65535.0),
            ("active_cycles", false, 75535.0),
            ("analytics", false, 85535.0),
            ("drafts", true, 95535.0),
            ("your_work", true, 105535.0),
            ("archives", false, 115535.0),
            ("stickies", true, 125535.0),
        ]
    );
}

/// Spec: the pin's `HomeWidgetKeys` without the two it never makes, `1000 - position` for the place (999, 998, 997), enabled and
/// with an empty config, and listed the way the pin lists them, which is the row it made last first (the fixture S24).
#[test]
fn a_new_members_home_page_is_the_pins_three_widgets_the_one_made_last_first() {
    let w = world();
    let rows = block_on(home_preferences(&w.app, &scope_of(&w, Actor::AdminA, Proj::A))).unwrap();
    let got: Vec<(&str, bool, &serde_json::Value, f64)> = rows.iter().map(|r| (r.key.as_str(), r.is_enabled, &r.config, r.sort_order)).collect();
    let empty = json!({});
    assert_eq!(got, [("my_stickies", true, &empty, 997.0), ("recents", true, &empty, 998.0), ("quick_links", true, &empty, 999.0)]);
}

/// Spec: the pin's `WorkspaceUserProperties` defaults (`apps/api/plane/db/models/workspace.py`): flat filters, display filters and
/// display properties each held one level deeper than the other lists hold them, no rich filters, ten projects in the navigation,
/// and an accordion (the fixture S22).
#[test]
fn a_new_members_list_settings_are_the_pins_defaults() {
    let w = world();
    let p = block_on(user_properties(&w.app, &scope_of(&w, Actor::AdminA, Proj::A))).unwrap();
    assert_eq!(
        p.filters,
        json!({"priority": null, "state": null, "state_group": null, "assignees": null, "created_by": null, "labels": null, "start_date": null, "target_date": null, "subscriber": null})
    );
    assert_eq!(
        p.display_filters,
        json!({"display_filters": {"group_by": null, "order_by": "-created_at", "type": null, "sub_issue": true, "show_empty_groups": true, "layout": "list", "calendar_date_range": ""}})
    );
    assert_eq!(
        p.display_properties,
        json!({"display_properties": {"assignee": true, "attachment_count": true, "created_on": true, "due_date": true, "estimate": true, "key": true, "labels": true, "link": true, "priority": true, "start_date": true, "state": true, "sub_issue_count": true, "updated_on": true}})
    );
    assert_eq!((p.rich_filters, p.navigation_project_limit, p.navigation_control_preference.as_str()), (json!({}), 10, "ACCORDION"));
}

// --- once, and for one member of one workspace --------------------------------------------------------------------------------

proptest! {
    #![proptest_config(proptest_config(64))]

    /// However many times a member visits, and whichever of the three kinds of row a visit reads, the answer is the first visit's,
    /// time included (the clock moves between visits: a row made again would show it). Another member's visit, or the same
    /// member's in the other workspace, has rows of its own and does not change these.
    #[test]
    fn prop_a_visit_is_answered_what_the_first_one_made(
        visits in prop::collection::vec((0..3usize, 0..2usize, 0..2usize), 1..16),
    ) {
        let w = ticking();
        let who = [Actor::AdminA, Actor::Dual];
        let proj = [Proj::A, Proj::B];
        // the first answer for each (kind, member, workspace)
        let mut first: std::collections::BTreeMap<(usize, usize, usize), String> = Default::default();
        for (kind, m, p) in visits {
            // Dual is in both workspaces; AdminA only in A, so a visit to B is Dual's
            let (m, p) = if who[m] == Actor::AdminA { (0, 0) } else { (m, p) };
            let scope = scope_of(&w, who[m], proj[p]);
            let answer = match kind {
                0 => format!("{:?}", block_on(sidebar_preferences(&w.app, &scope)).unwrap()),
                1 => format!("{:?}", block_on(home_preferences(&w.app, &scope)).unwrap()),
                _ => format!("{:?}", block_on(user_properties(&w.app, &scope)).unwrap()),
            };
            let was = first.entry((kind, m, p)).or_insert_with(|| answer.clone());
            prop_assert_eq!(&answer, &*was);
        }
    }
}

/// The settings row has an id of its own, made once: two members of one workspace and one member of two workspaces have three.
#[test]
fn the_list_settings_of_each_member_of_each_workspace_are_a_row_of_its_own() {
    let w = world();
    let ids: Vec<Uuid> = [(Actor::AdminA, Proj::A), (Actor::Dual, Proj::A), (Actor::Dual, Proj::B)]
        .iter()
        .map(|(a, p)| block_on(user_properties(&w.app, &scope_of(&w, *a, *p))).unwrap().id)
        .collect();
    assert_eq!(ids.iter().collect::<std::collections::BTreeSet<_>>().len(), 3, "{ids:?}");
}

// --- what a project made with no options is called -----------------------------------------------------------------------------

proptest! {
    #![proptest_config(proptest_config(256))]

    /// The key a project gets from its id: the last twelve hex digits, upper-cased, which is the lowest 48 bits of the id. (The
    /// migration that backfills a database made before the column does the same in SQL, `upper(split_part(id::text, '-', 5))`;
    /// `pg_adapter.rs` holds the two together.)
    #[test]
    fn prop_the_default_identifier_is_the_low_48_bits_of_the_id_in_upper_case_hex(n in any::<u128>()) {
        let id = ProjectId(Uuid::from_u128(n));
        prop_assert_eq!(default_project_identifier(id), format!("{:012X}", n & 0xFFFF_FFFF_FFFF));
    }
}

/// The two networks round-trip through their names, the names differ, and nothing else is a name.
#[test]
fn a_network_is_known_by_its_name_and_by_no_other() {
    for n in [ProjectNetwork::Secret, ProjectNetwork::Public] {
        assert_eq!(ProjectNetwork::parse(n.as_str()), Some(n));
    }
    assert_ne!(ProjectNetwork::Secret.as_str(), ProjectNetwork::Public.as_str());
    for other in ["", "private", "Secret", "PUBLIC", "0", "2"] {
        assert_eq!(ProjectNetwork::parse(other), None, "{other:?}");
    }
    assert_eq!(ProjectNetwork::default(), ProjectNetwork::Public, "a project made with no options is public");
}

// --- the project list ----------------------------------------------------------------------------------------------------------

/// A scope reaches the projects of its own workspace and no other, and each is listed with the caller's own role in it (none for
/// a member who is not in it): in this world every workspace member has the workspace role `member`, and the projects are public.
#[rstest]
fn the_project_list_is_the_scopes_workspace_with_the_callers_own_role(
    #[values(Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA, Actor::MemberB, Actor::Dual)] who: Actor,
    #[values(Proj::A, Proj::B)] proj: Proj,
) {
    let w = world();
    // the spec's table: who belongs to which workspace, and their role in its project
    let role = match (proj, who) {
        (Proj::A, Actor::AdminA) => Some(Some(Role::Admin)),
        (Proj::A, Actor::MemberA | Actor::Dual) => Some(Some(Role::Member)),
        (Proj::A, Actor::GuestA) => Some(Some(Role::Guest)),
        (Proj::A, Actor::OutsiderA) => Some(None),
        (Proj::B, Actor::MemberB) => Some(Some(Role::Member)),
        (Proj::B, Actor::Dual) => Some(Some(Role::Guest)),
        _ => None,
    };
    let scope = block_on(authorize_workspace(&w.app, &w.principal(who), &w.slug(proj, Slug::Own)));
    match role {
        None => assert!(matches!(scope, Err(CoreError::NotFound)), "{who:?} in {proj:?}"),
        Some(role) => {
            let listed = block_on(workspace_projects(&w.app, &scope.unwrap())).unwrap();
            let got: Vec<(ProjectId, WorkspaceId, Option<Role>)> = listed.iter().map(|l| (l.project.id, l.project.workspace_id, l.role)).collect();
            assert_eq!(got, [(w.project(proj), w.workspace_of(proj), role)], "{who:?} in {proj:?}");
        }
    }
}
