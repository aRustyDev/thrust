//! What the project routes (S30, S32, S34 to S46) derive from callers and rows that the recorded journey never has: a workspace
//! member who is not in the project, a project guest, a secret project, an archived one, two projects at once. The pin's answers for
//! those come from its source (the journey has one admin who is in every project they make), so these tests state the rules instead
//! of replaying a recording.
//!
//! What must hold: every row refuses no session with 401; each row refuses the people it refuses with the body the pin's guard for
//! it gives (`{"error"}` from the decorator, `{"detail"}` from a permission class, a 404 for a missing object); an admin or a member
//! of the workspace makes a project and is its admin, with the pin's 44 keys; the workspace's admins and the project's own admins
//! change one, nobody else, and not an archived one; a project is opened by its members and refused to the rest in three ways; the
//! counts, the states and the member rows are the pin's.

mod common;

use common::*;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{json, Value};
use thrust_contracts::*;
use thrust_ports::*;

// --- who may ask -------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    S34,
    S35,
    S36,
    S37,
    S38,
    S39,
    S40,
    S41,
    S42,
    S43,
    S44,
    S45,
    S46,
}

impl Row {
    fn path(self) -> String {
        match self {
            Row::S34 => project_path(""),
            Row::S35 => format!("/api/workspaces/{SLUG}/project-stats/"),
            Row::S36 => project_path("project-members/me/"),
            Row::S37 => project_path("user-properties/"),
            Row::S38 => project_path("issue-labels/"),
            Row::S39 => project_path("members/"),
            Row::S40 => project_path("states/"),
            Row::S41 => project_path("intake-state/"),
            Row::S42 => project_path("estimates/"),
            Row::S43 => project_path("cycles/"),
            Row::S44 => format!("/api/workspaces/{SLUG}/modules/"),
            Row::S45 => project_path("modules/"),
            Row::S46 => project_path("views/"),
        }
    }
}

fn by_decorator() -> Value {
    json!({"error": "You don't have the required permissions."})
}

fn by_class() -> Value {
    json!({"detail": "You do not have permission to perform this action."})
}

/// The spec's table, from the pin's guards (`allow_permission` and the permission classes; fixtures S34 to S46 record the creator, a
/// stranger and a project that is not there): what each caller is answered on each row. `None` is a 200.
fn refusal(row: Row, who: Caller, secret: bool) -> Option<(u16, Value)> {
    if who == Caller::Anonymous {
        return Some((401, json!({"detail": "Authentication credentials were not provided."})));
    }
    let (ws, project) = (who.in_workspace(), who.in_project());
    match row {
        Row::S34 if !ws => Some((403, by_decorator())),
        Row::S34 if project => None,
        Row::S34 if secret => Some((403, json!({"error": "You do not have permission"}))),
        Row::S34 => Some((409, json!({"error": "You are not a member of this project"}))),
        Row::S35 if !ws => Some((403, by_decorator())),
        Row::S44 if !ws => Some((403, by_class())),
        Row::S36 if !project => Some((404, json!({"error": "The required object does not exist."}))),
        // a permission class that lets in every member of the workspace and lists only what the caller is in
        Row::S38 if !ws => Some((403, by_class())),
        Row::S42 if !project => Some((403, by_class())),
        Row::S37 | Row::S39 | Row::S40 | Row::S41 | Row::S43 | Row::S45 | Row::S46 if !project => Some((403, by_decorator())),
        _ => None,
    }
}

#[rstest]
fn each_row_answers_each_kind_of_caller_as_the_pins_guard_does(
    #[values(Row::S34, Row::S35, Row::S36, Row::S37, Row::S38, Row::S39, Row::S40, Row::S41, Row::S42, Row::S43, Row::S44, Row::S45, Row::S46)] row: Row,
    #[values(
        Caller::Anonymous,
        Caller::Stranger,
        Caller::WsAdmin,
        Caller::WsMember,
        Caller::WsGuest,
        Caller::ProjectAdmin,
        Caller::ProjectMember,
        Caller::ProjectGuest
    )]
    who: Caller,
    #[values(false, true)] secret: bool,
) {
    let c = cast();
    if secret {
        let mut p = c.project_row();
        p.network = ProjectNetwork::Secret;
        c.save(&p);
    }
    let r = c.call(who, "GET", &row.path(), Value::Null);
    match refusal(row, who, secret) {
        Some((status, body)) => assert_eq!((r["status"].clone(), r["body"].clone()), (json!(status), body), "{row:?} {who:?} secret={secret}"),
        None => {
            assert_eq!(r["status"], 200, "{row:?} {who:?}: {r}");
            // a member of the workspace who is not in the project is shown none of the project's labels
            if row == Row::S38 && !who.in_project() {
                assert_eq!(r["body"], json!([]));
            }
        }
    }
}

/// A project that is not there: the refusals a member of the workspace who is not in it gets, and a 404 for S34 and S36.
#[rstest]
fn a_project_that_is_not_there_is_refused_as_one_the_caller_is_not_in(
    #[values(Row::S34, Row::S36, Row::S37, Row::S38, Row::S39, Row::S40, Row::S41, Row::S42, Row::S43, Row::S45, Row::S46)] row: Row,
) {
    let c = cast();
    let path = row.path().replace(&Uuid::from_u128(PROJECT).to_string(), &Uuid::from_u128(0xBAD).to_string());
    let r = c.call(Caller::ProjectAdmin, "GET", &path, Value::Null);
    let want = match row {
        Row::S34 => (404, json!({"error": "Project does not exist"})),
        Row::S36 => (404, json!({"error": "The required object does not exist."})),
        Row::S38 => (200, json!([])),
        Row::S42 => (403, by_class()),
        _ => (403, by_decorator()),
    };
    assert_eq!((r["status"].clone(), r["body"].clone()), (json!(want.0), want.1), "{row:?}");
}

// --- making a project --------------------------------------------------------------------------------------------------------

fn body(name: &str, identifier: &str) -> Value {
    json!({
        "name": name, "identifier": identifier, "network": 2, "description": "", "project_lead": null,
        "logo_props": {"in_use": "emoji", "emoji": {"value": "128512"}}, "cover_image_url": "/cover-images/image_5.webp",
    })
}

/// Spec (the pin's `create` decorator): an admin or a member of the workspace makes a project, a guest does not, and nor does
/// someone outside the workspace. A refusal makes nothing.
#[rstest]
fn who_may_make_a_project(
    #[values(
        Caller::Anonymous,
        Caller::Stranger,
        Caller::WsAdmin,
        Caller::WsMember,
        Caller::WsGuest,
        Caller::ProjectAdmin,
        Caller::ProjectMember,
        Caller::ProjectGuest
    )]
    who: Caller,
) {
    let c = cast();
    let r = c.call(who, "POST", &projects_path(), body("New One", "NEW"));
    match who {
        Caller::Anonymous => assert_eq!(r["status"], 401),
        Caller::Stranger | Caller::WsGuest | Caller::ProjectGuest => assert_eq!((r["status"].clone(), r["body"].clone()), (json!(403), by_decorator())),
        _ => {
            assert_eq!(r["status"], 201, "{r}");
            // the maker is the project's admin, and the only one in it
            assert_eq!((r["body"]["member_role"].clone(), r["body"]["members"].clone()), (json!(20), json!([c.user(who).0])));
        }
    }
    let listed = c.call(Caller::WsAdmin, "GET", &projects_path(), Value::Null);
    assert_eq!(listed["body"].as_array().unwrap().len(), if r["status"] == 201 { 2 } else { 1 }, "{listed}");
}

/// The 44 keys of the pin's `ProjectListSerializer` (the answer to a creation, an update and an opening), spelled out.
const PROJECT_KEYS: [&str; 44] = [
    "anchor", "archive_in", "archived_at", "close_in", "cover_image", "cover_image_asset", "cover_image_url", "created_at", "created_by",
    "cycle_view", "default_assignee", "default_state", "deleted_at", "description", "description_html", "description_text", "emoji", "estimate",
    "external_id", "external_source", "guest_view_all_features", "icon_prop", "id", "identifier", "inbox_view", "intake_view", "is_favorite",
    "is_issue_type_enabled", "is_time_tracking_enabled", "issue_views_view", "logo_props", "member_role", "members", "module_view", "name",
    "network", "next_work_item_sequence", "page_view", "project_lead", "sort_order", "timezone", "updated_at", "updated_by", "workspace",
];

#[test]
fn a_project_is_answered_with_the_44_keys_of_the_pins_list_serializer_on_creation_update_and_opening() {
    let c = cast();
    let made = c.call(Caller::WsMember, "POST", &projects_path(), body("New One", "NEW"));
    let path = format!("{}{}/", projects_path(), made["body"]["id"].as_str().unwrap());
    let updated = c.call(Caller::WsAdmin, "PATCH", &path, json!({"description": "d"}));
    let opened = c.call(Caller::WsMember, "GET", &path, Value::Null);
    let mut keys = PROJECT_KEYS.to_vec();
    keys.sort_unstable();
    for (what, r) in [("created", &made), ("updated", &updated), ("opened", &opened)] {
        let mut got: Vec<&str> = r["body"].as_object().unwrap_or_else(|| panic!("{what}: {r}")).keys().map(String::as_str).collect();
        got.sort_unstable();
        assert_eq!(got, keys, "{what}");
    }
}

/// Spec (the pin's `create`): the maker is the project's admin, and so is a lead who is someone else (listed first: theirs is the
/// newer row, and the pin lists a project's people newest first); each has their place in their own list, 65535 for a first
/// project in the workspace and 10000 less for each later one; the cover image's address is read only and ignored; and what the
/// lead then sees is the project's.
#[test]
fn a_lead_who_is_someone_else_is_made_an_admin_and_the_places_follow_the_lowest_held() {
    let c = cast();
    let lead = c.user(Caller::WsMember);
    let mut b = body("Led", "LED");
    b["project_lead"] = json!(lead.0);
    let made = c.call(Caller::WsAdmin, "POST", &projects_path(), b);
    assert_eq!(made["status"], 201, "{made}");
    let (maker, id) = (c.user(Caller::WsAdmin), made["body"]["id"].as_str().unwrap().to_string());
    assert_eq!(made["body"]["members"], json!([lead.0, maker.0]));
    assert_eq!((made["body"]["project_lead"].clone(), made["body"]["cover_image"].clone(), made["body"]["cover_image_url"].clone()), (json!(lead.0), Value::Null, Value::Null));
    // the lead is in it, as an admin
    let me = c.call(Caller::WsMember, "GET", &format!("{}{id}/project-members/me/", projects_path()), Value::Null);
    assert_eq!((me["status"].clone(), me["body"]["role"].clone()), (json!(200), json!(20)), "{me}");
    // the maker's first project is this one (65535); each later one is 10000 below the lowest held
    assert_eq!(made["body"]["sort_order"], json!(65535.0));
    let second = c.call(Caller::WsAdmin, "POST", &projects_path(), body("Second", "SEC"));
    assert_eq!(second["body"]["sort_order"], json!(55535.0), "{second}");
    let third = c.call(Caller::WsAdmin, "POST", &projects_path(), body("Third", "THI"));
    assert_eq!(third["body"]["sort_order"], json!(45535.0), "{third}");
}

/// Deviation D-026 (the pin lets any user lead a project): a lead who is a user but no member of the workspace is a field error in
/// Thrust's own words, on a creation and on a change, and nothing is made or changed; a member of the workspace is accepted, and
/// a change that names no lead does not ask.
#[test]
fn a_lead_who_is_no_member_of_the_workspace_is_a_field_error_and_changes_nothing() {
    let c = cast();
    let want = json!({"project_lead": ["The lead of a project must be a member of its workspace."]});
    let mut b = body("Led", "LED");
    b["project_lead"] = json!(c.user(Caller::Stranger).0);
    let made = c.call(Caller::WsAdmin, "POST", &projects_path(), b);
    assert_eq!((made["status"].clone(), made["body"].clone()), (json!(400), want.clone()), "{made}");
    let all = c.call(Caller::WsAdmin, "GET", &projects_path(), Value::Null);
    assert_eq!(all["body"].as_array().unwrap().len(), 1, "a refused creation makes nothing: {all}");

    let changed = c.call(Caller::ProjectAdmin, "PATCH", &project_path(""), json!({"project_lead": c.user(Caller::Stranger).0}));
    assert_eq!((changed["status"].clone(), changed["body"].clone()), (json!(400), want), "{changed}");
    let kept = c.call(Caller::ProjectAdmin, "GET", &project_path(""), Value::Null);
    assert_eq!(kept["body"]["project_lead"], Value::Null, "{kept}");

    let ok = c.call(Caller::ProjectAdmin, "PATCH", &project_path(""), json!({"project_lead": c.user(Caller::WsMember).0}));
    assert_eq!((ok["status"].clone(), ok["body"]["project_lead"].clone()), (json!(200), json!(c.user(Caller::WsMember).0)), "{ok}");
    let untouched = c.call(Caller::ProjectAdmin, "PATCH", &project_path(""), json!({"description": "d"}));
    assert_eq!((untouched["status"].clone(), untouched["body"]["project_lead"].clone()), (json!(200), json!(c.user(Caller::WsMember).0)), "{untouched}");
}

/// How a number of months is written in a request: the forms the pin's `IntegerField` reads (fixture S32 records the odd ones).
#[derive(Debug, Clone, Copy)]
enum Form {
    Int,
    WholeFloat,
    Text,
    TextWithZeros,
    TextPadded,
    TextSigned,
}

fn written(n: i64, form: Form) -> Value {
    match form {
        Form::Int => json!(n),
        Form::WholeFloat => json!(n as f64),
        Form::Text => json!(n.to_string()),
        Form::TextWithZeros => json!(format!("{n}.00")),
        Form::TextPadded => json!(format!(" {n} ")),
        Form::TextSigned => json!(if n >= 0 { format!("+{n}") } else { n.to_string() }),
    }
}

/// Every caller who is a member of the workspace, in some role.
fn workspace_members() -> impl Strategy<Value = Caller> {
    prop_oneof![
        Just(Caller::WsAdmin),
        Just(Caller::WsMember),
        Just(Caller::WsGuest),
        Just(Caller::ProjectAdmin),
        Just(Caller::ProjectMember),
        Just(Caller::ProjectGuest)
    ]
}

/// A JSON value that is no user's id, and none of the two things DRF reads as no lead (null and the empty string): integers that
/// no user's id can be (negative, or above 2^40), floats, text that is no UUID, booleans and lists of numbers.
fn no_lead_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<bool>().prop_map(Value::from),
        (i64::MIN..-1i64).prop_map(Value::from),
        ((1i64 << 40)..i64::MAX).prop_map(Value::from),
        (-1e6..1e6f64).prop_map(Value::from),
        "[ -~]{1,12}".prop_map(Value::from),
        prop::collection::vec(any::<i32>().prop_map(Value::from), 0..3).prop_map(Value::Array),
    ]
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// Spec (the pin's `IntegerField` with the bounds 0 and 12 on `archive_in` and `close_in`): a whole number from 0 to 12 is taken
    /// as an integer, a float with no fraction, or a string with spaces, a sign or `.00` after it; anything else whole is out of
    /// range, and each says so in DRF's words.
    #[test]
    fn prop_months_are_read_as_the_pin_reads_them(
        n in -4..20i64,
        form in prop_oneof![Just(Form::Int), Just(Form::WholeFloat), Just(Form::Text), Just(Form::TextWithZeros), Just(Form::TextPadded), Just(Form::TextSigned)],
    ) {
        let c = cast();
        let value = written(n, form);
        let r = c.call(Caller::ProjectAdmin, "PATCH", &project_path(""), json!({"archive_in": value, "close_in": value}));
        if (0..=12).contains(&n) {
            prop_assert_eq!(&r["status"], &json!(200), "{:?} {}", form, r);
            prop_assert_eq!((&r["body"]["archive_in"], &r["body"]["close_in"]), (&json!(n), &json!(n)));
        } else {
            let message = if n > 12 { "Ensure this value is less than or equal to 12." } else { "Ensure this value is greater than or equal to 0." };
            prop_assert_eq!(&r["status"], &json!(400), "{:?} {}", form, r);
            prop_assert_eq!(&r["body"], &json!({"archive_in": [message], "close_in": [message]}));
        }
    }

    /// What is no whole number is refused as no integer: a float with a fraction, text that is no number, a boolean, a list and an
    /// object. The project keeps the months it had.
    #[test]
    fn prop_months_that_are_no_whole_number_are_refused_as_no_integer(
        whole in -20..20i64,
        fraction in 1..10u32,
        text in "[a-z]{1,6}",
        kind in 0..7u8,
    ) {
        let c = cast();
        let value = match kind {
            0 => json!(whole as f64 + f64::from(fraction) / 10.0),
            1 => json!(text),
            2 => json!(whole > 0),
            3 => json!([whole]),
            4 => json!(format!("{whole}.{fraction}")),
            5 => json!(format!("{whole}.0{fraction}")),
            _ => Value::Object(std::iter::once((text.clone(), json!(whole))).collect()),
        };
        let r = c.call(Caller::ProjectAdmin, "PATCH", &project_path(""), json!({"archive_in": value}));
        prop_assert_eq!(&r["status"], &json!(400), "{}", r);
        prop_assert_eq!(&r["body"], &json!({"archive_in": ["A valid integer is required."]}));
        let kept = c.call(Caller::ProjectAdmin, "GET", &project_path(""), Value::Null);
        prop_assert_eq!(&kept["body"]["archive_in"], &json!(0));
    }

    /// A lead is found by any spelling of a member's id that Python's `uuid.UUID` reads: with or without hyphens, in braces, as a
    /// URN, in either case. They become the project's lead, written the one way.
    #[test]
    fn prop_a_lead_is_found_by_any_spelling_of_their_id(who in workspace_members(), spelling in 0..6u8) {
        let c = cast();
        let id = c.user(who).0;
        let (hyphens, simple) = (id.hyphenated().to_string(), id.simple().to_string());
        let sent = match spelling {
            0 => hyphens.clone(),
            1 => hyphens.to_uppercase(),
            2 => simple,
            3 => format!("{{{hyphens}}}"),
            4 => format!("urn:uuid:{hyphens}"),
            _ => id.simple().to_string().to_uppercase(),
        };
        let r = c.call(Caller::ProjectAdmin, "PATCH", &project_path(""), json!({"project_lead": sent}));
        prop_assert_eq!(&r["status"], &json!(200), "{} {}", sent, r);
        prop_assert_eq!(&r["body"]["project_lead"], &json!(hyphens));
    }

    /// A lead that is no user's id, whatever else it is, is refused with one message under `project_lead` (never a server error),
    /// and the project keeps its lead. Null and the empty string are no lead at all, and are taken.
    #[test]
    fn prop_a_lead_that_is_no_id_is_refused_whatever_it_is(value in no_lead_value(), blank in prop_oneof![Just(Value::Null), Just(json!(""))]) {
        let c = cast();
        let r = c.call(Caller::ProjectAdmin, "PATCH", &project_path(""), json!({"project_lead": value}));
        prop_assert_eq!(&r["status"], &json!(400), "{} {}", value, r);
        let errors = r["body"].as_object().unwrap();
        prop_assert_eq!(errors.keys().collect::<Vec<_>>(), vec!["project_lead"]);
        prop_assert_eq!(errors["project_lead"].as_array().map(Vec::len), Some(1));
        let none = c.call(Caller::ProjectAdmin, "PATCH", &project_path(""), json!({"project_lead": blank}));
        prop_assert_eq!((&none["status"], &none["body"]["project_lead"]), (&json!(200), &Value::Null), "{}", none);
    }
}

/// Deviation D-027: the logo column holds an object, so a logo that is anything else is a field error in Thrust's words (the pin
/// stores any value; the app sends an object). Null is refused as the pin refuses it.
#[rstest]
#[case::a_string(json!("x"), 400, "The logo of a project must be a JSON object.")]
#[case::a_list(json!([1]), 400, "The logo of a project must be a JSON object.")]
#[case::a_number(json!(5), 400, "The logo of a project must be a JSON object.")]
#[case::null(Value::Null, 400, "This field may not be null.")]
fn a_logo_that_is_not_an_object_is_a_field_error(#[case] logo: Value, #[case] status: u16, #[case] message: &str) {
    let c = cast();
    let r = c.call(Caller::ProjectAdmin, "PATCH", &project_path(""), json!({"logo_props": logo}));
    assert_eq!((r["status"].clone(), r["body"].clone()), (json!(status), json!({"logo_props": [message]})), "{r}");
    let ok = c.call(Caller::ProjectAdmin, "PATCH", &project_path(""), json!({"logo_props": {"in_use": "emoji"}}));
    assert_eq!((ok["status"].clone(), ok["body"]["logo_props"].clone()), (json!(200), json!({"in_use": "emoji"})), "{ok}");
}

fn name_strategy() -> impl Strategy<Value = String> {
    "[ ]{0,2}[A-Za-z0-9][A-Za-z0-9 _/]{0,20}[ ]{0,2}"
}

fn key_strategy() -> impl Strategy<Value = String> {
    "[ ]{0,2}[A-Z0-9]{1,12}[ ]{0,2}"
}

proptest! {
    #![proptest_config(proptest_config(48))]

    /// A project with a name and a key that hold no forbidden character is made with both trimmed, and a second with the same
    /// trimmed name or key is refused with the pin's code for the field that clashed (both, when both do), and makes nothing.
    #[test]
    fn prop_a_valid_name_and_key_are_made_trimmed_and_a_second_with_either_is_refused(
        name in name_strategy(), key in key_strategy(), other_name in name_strategy(), other_key in key_strategy(),
    ) {
        let c = cast();
        let (n, k) = (name.trim().to_string(), key.trim().to_string());
        // the cast's own project is named `Cast Project` and keyed `PRJ`: leave them out of this property
        prop_assume!(n != "Cast Project" && k != "PRJ" && other_name.trim() != "Cast Project" && other_key.trim() != "PRJ");
        let first = c.call(Caller::WsMember, "POST", &projects_path(), body(&name, &key));
        prop_assert_eq!(&first["status"], &json!(201), "{}", first);
        prop_assert_eq!((&first["body"]["name"], &first["body"]["identifier"]), (&json!(n), &json!(k)));
        let second = c.call(Caller::WsMember, "POST", &projects_path(), body(&other_name, &other_key));
        let (name_clash, key_clash) = (other_name.trim() == n, other_key.trim() == k);
        if name_clash || key_clash {
            let mut want = serde_json::Map::new();
            if name_clash {
                want.insert("name".into(), json!(["PROJECT_NAME_ALREADY_EXIST"]));
            }
            if key_clash {
                want.insert("identifier".into(), json!(["PROJECT_IDENTIFIER_ALREADY_EXIST"]));
            }
            prop_assert_eq!(&second["status"], &json!(400));
            prop_assert_eq!(&second["body"], &Value::Object(want));
        } else {
            prop_assert_eq!(&second["status"], &json!(201), "{}", second);
        }
        let all = c.call(Caller::WsAdmin, "GET", &projects_path(), Value::Null);
        prop_assert_eq!(all["body"].as_array().unwrap().len(), if name_clash || key_clash { 2 } else { 3 });
    }
}

// --- changing and opening ----------------------------------------------------------------------------------------------------

/// Spec (the pin's `partial_update`): the workspace's admins and the project's own admins change a project (the 403's body
/// `{"error"}` is the view's own), and nobody else in the workspace does.
#[rstest]
fn who_may_change_a_project(
    #[values(Caller::Stranger, Caller::WsAdmin, Caller::WsMember, Caller::WsGuest, Caller::ProjectAdmin, Caller::ProjectMember, Caller::ProjectGuest)] who: Caller,
) {
    let c = cast();
    let r = c.call(who, "PATCH", &project_path(""), json!({"description": "changed by someone"}));
    if matches!(who, Caller::WsAdmin | Caller::ProjectAdmin) {
        assert_eq!((r["status"].clone(), r["body"]["description"].clone(), r["body"]["updated_by"].clone()), (json!(200), json!("changed by someone"), json!(c.user(who).0)), "{r}");
    } else {
        assert_eq!((r["status"].clone(), r["body"].clone()), (json!(403), by_decorator()), "{who:?}");
        let after = c.call(Caller::ProjectAdmin, "GET", &project_path(""), Value::Null);
        assert_eq!(after["body"]["description"], json!(""), "a refused change changed the project");
    }
}

/// A project that is archived is not changed (whatever the body says, even a bad one), is not opened, and is still listed, with its
/// archive time, to the workspace's admin.
#[test]
fn an_archived_project_is_not_changed_or_opened_and_is_still_listed() {
    let c = cast();
    let mut row = c.project_row();
    row.archived_at = Some(at(500));
    c.save(&row);
    for patch in [json!({}), json!({"description": "x"}), json!({"name": "bad.name"})] {
        let r = c.call(Caller::ProjectAdmin, "PATCH", &project_path(""), patch);
        assert_eq!((r["status"].clone(), r["body"].clone()), (json!(400), json!({"error": "Archived projects cannot be updated"})));
    }
    let r = c.call(Caller::ProjectAdmin, "GET", &project_path(""), Value::Null);
    assert_eq!((r["status"].clone(), r["body"].clone()), (json!(404), json!({"error": "Project does not exist"})));
    let listed = c.call(Caller::WsAdmin, "GET", &projects_path(), Value::Null);
    assert!(listed["body"][0]["archived_at"].is_string(), "{listed}");
}

/// A patch sets what it names (DRF's own readings: a string `"0"` is a network, `"false"` a boolean, `"6"` months), `inbox_view` is
/// what sets the intake view on a change (and is read only on a creation, where `intake_view` is the column), and an `id` in the body
/// does not move the project.
#[test]
fn a_patch_sets_what_it_names_and_the_inbox_view_is_the_intake_view() {
    let c = cast();
    let r = c.call(
        Caller::ProjectAdmin,
        "PATCH",
        &project_path(""),
        json!({"name": "  Renamed  ", "network": "0", "inbox_view": true, "page_view": "false", "archive_in": "6", "timezone": "Asia/Tokyo", "emoji": null, "logo_props": {"in_use": "icon"}, "id": "ignored"}),
    );
    assert_eq!(r["status"], 200, "{r}");
    let b = &r["body"];
    assert_eq!((b["name"].clone(), b["network"].clone(), b["intake_view"].clone(), b["inbox_view"].clone()), (json!("Renamed"), json!(0), json!(true), json!(true)));
    assert_eq!((b["page_view"].clone(), b["archive_in"].clone(), b["timezone"].clone(), b["logo_props"].clone()), (json!(false), json!(6), json!("Asia/Tokyo"), json!({"in_use": "icon"})));
    assert_eq!(b["id"], json!(Uuid::from_u128(PROJECT)), "a patch does not move a project");
    let made = c.call(Caller::WsAdmin, "POST", &projects_path(), json!({"name": "A", "identifier": "A", "inbox_view": true}));
    assert_eq!(made["body"]["intake_view"], json!(false));
    let made = c.call(Caller::WsAdmin, "POST", &projects_path(), json!({"name": "B", "identifier": "B", "intake_view": true}));
    assert_eq!(made["body"]["intake_view"], json!(true));
}

// --- what the app reads about a project --------------------------------------------------------------------------------------

#[test]
fn the_members_states_and_settings_of_a_project_are_the_pins() {
    let c = cast();
    let made = c.call(Caller::WsMember, "POST", &projects_path(), body("New One", "NEW"));
    let id = made["body"]["id"].as_str().unwrap().to_string();
    let p = |tail: &str| format!("{}{id}/{tail}", projects_path());
    // S39: the one admin, whose row names them as member and role 20 both ways
    let members = c.call(Caller::WsMember, "GET", &p("members/"), Value::Null);
    assert_eq!(members["body"][0]["member"], json!(c.user(Caller::WsMember).0), "{members}");
    assert_eq!((members["body"][0]["role"].clone(), members["body"][0]["original_role"].clone()), (json!(20), json!(20)));
    // S40: the five states, by sequence, each the only one of its group (so `order` is 1), and not the intake state
    let states = c.call(Caller::WsMember, "GET", &p("states/"), Value::Null);
    let names: Vec<&str> = states["body"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Backlog", "Todo", "In Progress", "Done", "Cancelled"]);
    assert!(states["body"].as_array().unwrap().iter().all(|s| s["order"] == json!(1.0)));
    assert_eq!(states["body"][0]["default"], json!(true));
    // S41: the intake state, which has no `order`
    let triage = c.call(Caller::WsMember, "GET", &p("intake-state/"), Value::Null);
    assert_eq!((triage["body"]["group"].clone(), triage["body"]["sequence"].clone(), triage["body"].get("order").is_none()), (json!("triage"), json!(65000.0), true), "{triage}");
    // S36: the row with its settings filled in with the pin's defaults
    let me = c.call(Caller::WsMember, "GET", &p("project-members/me/"), Value::Null);
    assert_eq!(me["body"]["view_props"]["display_filters"]["layout"], json!("list"), "{me}");
    assert_eq!(me["body"]["preferences"]["navigation"]["default_tab"], json!("work_items"));
    assert_eq!((me["body"]["workspace"]["slug"].clone(), me["body"]["project"]["identifier"].clone(), me["body"]["is_active"].clone()), (json!(SLUG), json!("NEW"), json!(true)));
    assert_eq!(me["body"]["member"]["id"], json!(c.user(Caller::WsMember).0));
    // S37: the creator's settings are there from the start and asking again is the same row
    let props = c.call(Caller::WsMember, "GET", &p("user-properties/"), Value::Null);
    assert_eq!(c.call(Caller::WsMember, "GET", &p("user-properties/"), Value::Null)["body"], props["body"]);
    assert_eq!((props["body"]["sort_order"].clone(), props["body"]["user"].clone()), (json!(65535.0), json!(c.user(Caller::WsMember).0)));
}

/// A member of the seeded project has no settings row until they ask, and then has the pin's defaults, made once.
#[test]
fn a_member_with_no_settings_row_gets_one_on_asking() {
    let c = cast();
    let first = c.call(Caller::ProjectMember, "GET", &project_path("user-properties/"), Value::Null);
    assert_eq!((first["status"].clone(), first["body"]["sort_order"].clone(), first["body"]["filters"]["state"].clone()), (json!(200), json!(65535.0), Value::Null), "{first}");
    assert_eq!(first["body"]["display_filters"]["layout"], json!("list"));
    assert_eq!(c.call(Caller::ProjectMember, "GET", &project_path("user-properties/"), Value::Null)["body"], first["body"]);
}

/// The intake state of a project that has none is the pin's 404 (a project made here always has one).
#[test]
fn a_project_with_no_intake_state_is_a_404() {
    let c = cast();
    let (ws, bare) = (WorkspaceId(Uuid::from_u128(WS)), ProjectId(Uuid::from_u128(0xA2)));
    seed(
        &c.db,
        SeedFixture {
            projects: vec![SeedProject { id: bare, workspace_id: ws, name: "Bare".into() }],
            memberships: vec![SeedMembership { project_id: bare, workspace_id: ws, user_id: c.user(Caller::ProjectMember), role: Role::Member }],
            ..SeedFixture::default()
        },
    );
    let path = |tail: &str| format!("{}{}/{tail}", projects_path(), Uuid::from_u128(0xA2));
    let r = c.call(Caller::ProjectMember, "GET", &path("intake-state/"), Value::Null);
    assert_eq!((r["status"].clone(), r["body"].clone()), (json!(404), json!({"error": "Triage state not found"})));
    assert_eq!(c.call(Caller::ProjectMember, "GET", &path("states/"), Value::Null)["body"], json!([]));
}

/// Spec (S20): the workspace's states are those of every project the caller is in, by `sequence`, and each one's `order` is its place
/// among the listed states of its group over how many there are: with two projects, 0.5 and 1.0.
#[test]
fn the_workspaces_states_count_every_project_the_caller_is_in_for_their_order() {
    let c = cast();
    assert_eq!(c.call(Caller::ProjectAdmin, "POST", &projects_path(), body("Two", "TWO"))["status"], 201);
    assert_eq!(c.call(Caller::ProjectAdmin, "POST", &projects_path(), body("Three", "THR"))["status"], 201);
    let path = format!("/api/workspaces/{SLUG}/states/");
    let states = c.call(Caller::ProjectAdmin, "GET", &path, Value::Null);
    let list = states["body"].as_array().unwrap();
    // the seeded project has its default state, the two made ones five each
    assert_eq!(list.len(), 11);
    let seqs: Vec<f64> = list.iter().map(|s| s["sequence"].as_f64().unwrap()).collect();
    assert!(seqs.windows(2).all(|w| w[0] <= w[1]), "{seqs:?}");
    // three projects have a backlog state, so its three are a third, two thirds and all of them; the others are alone in theirs
    let mut orders: Vec<f64> = list.iter().filter(|s| s["group"] == "backlog").map(|s| s["order"].as_f64().unwrap()).collect();
    orders.sort_by(f64::total_cmp);
    assert_eq!(orders, [1.0 / 3.0, 2.0 / 3.0, 3.0 / 3.0]);
    assert!(list.iter().filter(|s| s["group"] == "started").all(|s| s["order"].as_f64().unwrap() <= 1.0));
    // a member who is in none of the projects sees none of them
    assert_eq!(c.call(Caller::WsMember, "GET", &path, Value::Null)["body"], json!([]));
    // and the guest of the seeded project sees only its one state
    assert_eq!(c.call(Caller::ProjectGuest, "GET", &path, Value::Null)["body"].as_array().unwrap().len(), 1);
}

/// S35: the counts name every project of the workspace whoever asks, and `fields` and `project_ids` narrow them.
#[test]
fn the_project_counts_name_every_project_and_the_fields_and_ids_asked_for() {
    let c = cast();
    c.call(Caller::WsMember, "POST", &projects_path(), body("New One", "NEW"));
    let path = format!("/api/workspaces/{SLUG}/project-stats/");
    for who in [Caller::WsGuest, Caller::ProjectAdmin] {
        let all = c.call(who, "GET", &path, Value::Null);
        assert_eq!(all["body"].as_array().unwrap().len(), 2, "{who:?}");
        assert_eq!(all["body"][0].as_object().unwrap().len(), 6, "an id and the five counts");
    }
    let one = c.call(Caller::WsGuest, "GET", &format!("{path}?project_ids={}&fields=total_members,nothing", Uuid::from_u128(PROJECT)), Value::Null);
    assert_eq!(one["body"], json!([{"id": Uuid::from_u128(PROJECT), "total_members": 3}]), "{one}");
    // the work items it holds are counted (two live ones), and completed ones, cycles and modules are none yet (D-021)
    let item = |n: u128| SeedWorkItem {
        id: WorkItemId(Uuid::from_u128(n)),
        name: format!("w{n}"),
        description: None,
        created_by: c.user(Caller::WsAdmin),
        created_at: thrust_contracts::chrono::DateTime::from_timestamp(1_760_000_000, 0).unwrap(),
    };
    futures_executor::block_on(c.db.seed_work_items(&SystemScope::mint(), WorkspaceId(Uuid::from_u128(WS)), ProjectId(Uuid::from_u128(PROJECT)), &[item(1), item(2)])).unwrap();
    let counted = c.call(Caller::WsGuest, "GET", &format!("{path}?project_ids={}", Uuid::from_u128(PROJECT)), Value::Null);
    assert_eq!(
        counted["body"],
        json!([{"id": Uuid::from_u128(PROJECT), "total_issues": 2, "completed_issues": 0, "total_members": 3, "total_cycles": 0, "total_modules": 0}]),
        "{counted}"
    );
    let none = c.call(Caller::WsGuest, "GET", &format!("{path}?fields=nothing"), Value::Null);
    assert_eq!(none["body"][0].as_object().unwrap().len(), 6, "fields that are no count mean all of them");
    let bad = c.call(Caller::WsGuest, "GET", &format!("{path}?project_ids=nope"), Value::Null);
    assert_eq!((bad["status"].clone(), bad["body"].clone()), (json!(400), json!({"error": "Please provide valid detail"})));
}

/// S38: the labels of a project, by `sort_order`, for its members; a member of the workspace who is not in it is told `[]`.
#[test]
fn a_projects_labels_are_listed_by_sort_order_to_its_members_only() {
    let c = cast();
    let (ws, project) = (WorkspaceId(Uuid::from_u128(WS)), ProjectId(Uuid::from_u128(PROJECT)));
    let label = |n: u128, name: &str, sort_order: f64| SeedLabel {
        id: Uuid::from_u128(n),
        project_id: project,
        workspace_id: ws,
        name: name.into(),
        color: "#fff".into(),
        parent_id: None,
        sort_order,
    };
    seed(&c.db, SeedFixture { labels: vec![label(1, "later", 20.0), label(2, "first", 10.0)], ..SeedFixture::default() });
    let mine = c.call(Caller::ProjectGuest, "GET", &project_path("issue-labels/"), Value::Null);
    let names: Vec<&str> = mine["body"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["first", "later"]);
    assert_eq!(mine["body"][0], json!({"id": Uuid::from_u128(2), "project_id": project.0, "workspace_id": ws.0, "parent": null, "name": "first", "color": "#fff", "sort_order": 10.0}));
    assert_eq!(c.call(Caller::WsMember, "GET", &project_path("issue-labels/"), Value::Null)["body"], json!([]));
}
