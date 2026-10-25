//! What the work item routes (S47, S48, X01) derive from callers and rows that the recorded journey never has: a project guest, a
//! workspace member who is not in the project, a project that shows its guests everything or does not, many work items at once.
//! The pin's answers for those come from its source, so these tests state the rules instead of replaying a recording.
//!
//! What must hold: no session is 401 on every row; whoever is not a member of the project (or is a guest, for a creation) is
//! refused with the decorator's body; a guest reads only what they made unless the project shows them everything, and says why when
//! they open another's; a creation is read the way Django REST Framework reads it, each field in its own words; a page's envelope
//! is arithmetic over the number of work items, the page size and the page; what the list does not answer (grouping, filtering) it
//! refuses in its own words rather than answer wrongly.

mod common;

use common::*;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{json, Value};
use thrust_contracts::*;
use thrust_ports::*;

fn issues_path() -> String {
    project_path("issues/")
}

fn issue_path(id: &str) -> String {
    project_path(&format!("issues/{id}/"))
}

const UNAUTHENTICATED: &str = "Authentication credentials were not provided.";

fn by_decorator() -> Value {
    json!({"error": "You don't have the required permissions."})
}

/// Make a work item as `who` and answer its id.
fn make(c: &Cast, who: Caller, body: Value) -> String {
    let r = c.call(who, "POST", &issues_path(), body);
    assert_eq!(r["status"], 201, "{r}");
    r["body"]["id"].as_str().unwrap().to_string()
}

/// A work item a person made without a route: a guest cannot make one, but a member who is made a guest keeps theirs.
fn seed_item_by(c: &Cast, who: Caller, name: &str) -> String {
    let (ws, project) = (WorkspaceId(Uuid::from_u128(WS)), ProjectId(Uuid::from_u128(PROJECT)));
    let scope = Scope::mint(ws, project, c.user(who));
    let mut tx = futures_executor::block_on(c.db.begin(&scope)).unwrap();
    let alloc = futures_executor::block_on(tx.allocate(&scope)).unwrap();
    let id = WorkItemId(Uuid::from_u128(0x9000 + alloc.sequence_id as u128));
    let item = WorkItem {
        id,
        workspace_id: ws,
        project_id: project,
        sequence_id: alloc.sequence_id,
        sort_order: alloc.sort_order,
        name: name.into(),
        description: None,
        external_source: None,
        external_id: None,
        created_by: c.user(who),
        created_at: at(1),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    };
    futures_executor::block_on(tx.insert_work_item(&scope, &item)).unwrap();
    futures_executor::block_on(tx.commit()).unwrap();
    id.0.to_string()
}

fn show_guests_everything(c: &Cast, on: bool) {
    let mut row = c.project_row();
    row.guest_view_all_features = on;
    c.save(&row);
}

// --- who may ask -------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    S47,
    S48,
    X01,
}

/// Spec (the pin's `allow_permission` on the three views): no session is 401 everywhere; everyone who is not in the project is the
/// decorator's 403 (a workspace's admin among them, who must be in the project as well), and a guest is too, for a creation; a
/// guest who may read reads nothing here, because the work item they open was not made by them (the next test is theirs).
#[rstest]
fn each_row_answers_each_kind_of_caller_as_the_pins_guard_does(
    #[values(Row::S47, Row::S48, Row::X01)] row: Row,
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
    let existing = make(&c, Caller::ProjectAdmin, json!({"name": "made by the admin"}));
    let r = match row {
        Row::S47 => c.call(who, "GET", &issues_path(), Value::Null),
        Row::S48 => c.call(who, "POST", &issues_path(), json!({"name": "new"})),
        Row::X01 => c.call(who, "GET", &issue_path(&existing), Value::Null),
    };
    let (status, body) = (r["status"].as_u64().unwrap(), r["body"].clone());
    match (who, row) {
        (Caller::Anonymous, _) => assert_eq!((status, body), (401, json!({"detail": UNAUTHENTICATED}))),
        (Caller::ProjectAdmin | Caller::ProjectMember, Row::S48) => assert_eq!(status, 201, "{body}"),
        (Caller::ProjectAdmin | Caller::ProjectMember, _) => assert_eq!(status, 200, "{body}"),
        // a guest reads what they made: the page is empty, and the admin's work item is not theirs to open
        (Caller::ProjectGuest, Row::S47) => assert_eq!((status, body["results"].clone()), (200, json!([]))),
        (Caller::ProjectGuest, Row::X01) => assert_eq!((status, body), (403, json!({"error": "You are not allowed to view this issue"}))),
        _ => assert_eq!((status, body), (403, by_decorator()), "{who:?} {row:?}"),
    }
}

/// Spec (`retrieve` and `list`): a project guest reads their own work items whatever the project shows; the others' only when it
/// shows guests everything.
#[rstest]
fn a_guest_reads_their_own_and_the_others_only_when_the_project_shows_them(#[values(false, true)] shows_all: bool) {
    let c = cast();
    show_guests_everything(&c, shows_all);
    let others = make(&c, Caller::ProjectMember, json!({"name": "a member's"}));
    let own = seed_item_by(&c, Caller::ProjectGuest, "the guest's");
    let page = c.call(Caller::ProjectGuest, "GET", &issues_path(), Value::Null);
    let ids: Vec<&str> = page["body"]["results"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), if shows_all { 2 } else { 1 });
    assert!(ids.contains(&own.as_str()));
    assert_eq!(page["body"]["total_count"], ids.len());
    assert_eq!(c.call(Caller::ProjectGuest, "GET", &issue_path(&own), Value::Null)["status"], 200);
    let theirs = c.call(Caller::ProjectGuest, "GET", &issue_path(&others), Value::Null);
    assert_eq!(theirs["status"], if shows_all { 200 } else { 403 }, "{theirs}");
    // a work item that is not there is a 404 to them as to anyone
    let gone = c.call(Caller::ProjectGuest, "GET", &issue_path("00000000-0000-4000-8000-000000000bad"), Value::Null);
    assert_eq!((gone["status"].clone(), gone["body"].clone()), (json!(404), json!({"error": "The required object does not exist."})));
}

// --- making one ----------------------------------------------------------------------------------------------------------------

/// A body for a creation with `name` and one more field.
fn with(field: &str, value: Value) -> Value {
    let mut body = json!({"name": "n"});
    body[field] = value;
    body
}

/// Spec (DRF `CharField`, 1 to 255 characters once trimmed): text is kept trimmed; a number is its text; null, a bool, a list and
/// an object are refused, in DRF's words; an empty or blank name and one over 255 characters are refused.
#[test]
fn a_name_is_read_as_drf_reads_a_char_field() {
    let c = cast();
    let name_of = |v: Value| -> (u64, Value) {
        let r = c.call(Caller::ProjectMember, "POST", &issues_path(), json!({"name": v}));
        (r["status"].as_u64().unwrap(), if r["status"] == 201 { r["body"]["name"].clone() } else { r["body"].clone() })
    };
    assert_eq!(name_of(json!("  padded  ")), (201, json!("padded")));
    assert_eq!(name_of(json!(5)), (201, json!("5")));
    assert_eq!(name_of(json!("é".repeat(255))), (201, json!("é".repeat(255))));
    let refused = |v: Value, message: &str| assert_eq!(name_of(v.clone()), (400, json!({"name": [message]})), "{v}");
    refused(json!("é".repeat(256)), "Ensure this field has no more than 255 characters.");
    refused(json!(""), "This field may not be blank.");
    refused(json!("   "), "This field may not be blank.");
    refused(json!(null), "This field may not be null.");
    refused(json!(true), "Not a valid string.");
    refused(json!(["x"]), "Not a valid string.");
    refused(json!({"a": 1}), "Not a valid string.");
    let none = c.call(Caller::ProjectMember, "POST", &issues_path(), json!({}));
    assert_eq!(none["body"], json!({"name": ["This field is required."]}));
}

proptest! {
    #![proptest_config(proptest_config(48))]

    /// Spec (DRF `DateField`, which reads a date as Django's `parse_date` does): a day that exists, as `YYYY-MM-DD`, with or without
    /// the zeros of the month and the day, as `YYYYMMDD`, or as its ISO week (`YYYY-Www-D`), is read and kept written in full; the
    /// same day with a time after it, as `DD/MM/YYYY`, or one that does not exist, is refused with one message.
    #[test]
    fn prop_a_date_is_a_day_written_year_month_day(
        year in 1990..2100i32,
        month in 0..14u32,
        day in 0..33u32,
        form in 0..6u8,
    ) {
        use thrust_contracts::chrono::Datelike;
        let c = cast();
        let full = format!("{year:04}-{month:02}-{day:02}");
        let real_day = thrust_contracts::chrono::NaiveDate::from_ymd_opt(year, month, day);
        let real = real_day.is_some();
        let text = match form {
            0 => full.clone(),
            1 => format!("{full}T00:00:00Z"),
            2 => format!("{day:02}/{month:02}/{year:04}"),
            3 => format!("{year}-{month}-{day}"),
            4 => format!("{year:04}{month:02}{day:02}"),
            // the week form, of a day that exists (one that does not has no week: it stays the day, refused as it is)
            _ => real_day.map_or(full.clone(), |d| format!("{:04}-W{:02}-{}", d.iso_week().year(), d.iso_week().week(), d.weekday().number_from_monday())),
        };
        let r = c.call(Caller::ProjectMember, "POST", &issues_path(), with("start_date", json!(text)));
        if matches!(form, 0 | 3 | 4 | 5) && real {
            prop_assert_eq!((&r["status"], &r["body"]["start_date"]), (&json!(201), &json!(full)), "{}", r);
        } else {
            prop_assert_eq!(&r["status"], &json!(400), "{}", r);
            prop_assert_eq!(&r["body"], &json!({"start_date": ["Date has wrong format. Use one of these formats instead: YYYY-MM-DD."]}));
        }
    }

    /// Spec (DRF `ChoiceField`): the five priorities are accepted and come back; any other text is refused naming it; null is
    /// refused as null.
    #[test]
    fn prop_a_priority_is_one_of_the_five(pick in 0..8usize, junk in "[a-z]{1,8}") {
        let c = cast();
        let value = ["urgent", "high", "medium", "low", "none"].get(pick).map_or(junk.clone(), |p| (*p).to_string());
        let r = c.call(Caller::ProjectMember, "POST", &issues_path(), with("priority", json!(value)));
        if pick < 5 {
            prop_assert_eq!((&r["status"], &r["body"]["priority"]), (&json!(201), &json!(value)), "{}", r);
        } else if ["urgent", "high", "medium", "low", "none"].contains(&junk.as_str()) {
            prop_assert_eq!(&r["status"], &json!(201));
        } else {
            prop_assert_eq!((&r["status"], &r["body"]), (&json!(400), &json!({"priority": [format!("\"{junk}\" is not a valid choice.")]})));
        }
    }

    /// Spec (DRF `ListField` of related keys): a list of ids is read, each member on its own: a member that is no id is an error at
    /// its place whatever the others are; a value that is no list is told by Python's name for its type.
    #[test]
    fn prop_a_list_of_keys_is_read_member_by_member(bad_at in prop::option::of(0..4usize), len in 1..5usize, kind in 0..3u8, not_a_list in 0..5u8) {
        let c = cast();
        let who = c.user(Caller::ProjectMember).0.to_string();
        let mut items: Vec<Value> = (0..len).map(|_| json!(who)).collect();
        // a member that is no key: text that is no UUID, null, or the empty string (DRF reads it as null, which a member may not be)
        let (bad, message) = match kind {
            0 => (json!("not-an-id"), "\u{201c}not-an-id\u{201d} is not a valid UUID."),
            1 => (Value::Null, "This field may not be null."),
            _ => (json!(""), "This field may not be null."),
        };
        if let Some(i) = bad_at.filter(|i| *i < len) {
            items[i] = bad;
        }
        let r = c.call(Caller::ProjectMember, "POST", &issues_path(), with("assignee_ids", json!(items)));
        match bad_at.filter(|i| *i < len) {
            None => prop_assert_eq!(&r["status"], &json!(201), "{}", r),
            Some(i) => {
                prop_assert_eq!(&r["status"], &json!(400));
                prop_assert_eq!(&r["body"], &json!({"assignee_ids": {i.to_string(): [message]}}));
            }
        }
        let (value, name) = match not_a_list {
            0 => (json!("x"), "str"),
            1 => (json!(5), "int"),
            2 => (json!(1.5), "float"),
            3 => (json!(true), "bool"),
            _ => (json!({"a": 1}), "dict"),
        };
        let r = c.call(Caller::ProjectMember, "POST", &issues_path(), with("label_ids", value));
        prop_assert_eq!(&r["body"], &json!({"label_ids": [format!("Expected a list of items but got type \"{name}\".")]}));
        // null is refused as null, not as a type
        let r = c.call(Caller::ProjectMember, "POST", &issues_path(), with("label_ids", Value::Null));
        prop_assert_eq!(&r["body"], &json!({"label_ids": ["This field may not be null."]}));
    }

    /// Spec (the pin's `validate_html_content`): the text in a description's markup is kept and what can run is not: a script,
    /// a handler, a `javascript:` link. Detail shows what was stored.
    #[test]
    fn prop_a_description_keeps_its_text_and_loses_what_can_run(text in "[a-zA-Z0-9 ]{0,30}", attack in 0..4u8) {
        let c = cast();
        let injected = match attack {
            0 => format!("<p>{text}</p><script>alert(1)</script>"),
            1 => format!("<p onclick=\"x()\">{text}</p>"),
            2 => format!("<a href=\"javascript:alert(1)\">{text}</a>"),
            _ => format!("<p>{text}</p>"),
        };
        let id = make(&c, Caller::ProjectMember, with("description_html", json!(injected)));
        let stored = c.call(Caller::ProjectMember, "GET", &issue_path(&id), Value::Null)["body"]["description_html"].as_str().unwrap().to_string();
        for forbidden in ["<script", "onclick", "javascript:"] {
            prop_assert!(!stored.contains(forbidden), "{forbidden} in {stored}");
        }
        prop_assert!(stored.contains(text.as_str()), "{text:?} is gone from {stored:?}");
    }
}

/// Spec: the fields the pin sets itself or ignores are ignored, and the three it takes as relations to rows that do not exist here
/// take only null (deviation D-028): a parent, an estimate; a cycle, a module and a type are ignored.
#[test]
fn what_the_pin_sets_itself_is_ignored_and_what_has_nothing_behind_it_is_refused() {
    let c = cast();
    let nobody = "00000000-0000-4000-8000-00000000dead";
    let body = json!({
        "name": "ignored", "sort_order": 5, "sequence_id": 99, "completed_at": "2026-01-01T00:00:00Z", "created_by": nobody,
        "project_id": nobody, "type_id": nobody, "cycle_id": nobody, "module_ids": [nobody], "is_draft": true, "archived_at": "2026-01-01",
    });
    let r = c.call(Caller::ProjectMember, "POST", &issues_path(), body);
    assert_eq!(r["status"], 201, "{r}");
    assert_eq!((r["body"]["sequence_id"].clone(), r["body"]["sort_order"].clone(), r["body"]["is_draft"].clone()), (json!(1), json!(65535.0), json!(false)));
    assert_eq!((r["body"]["completed_at"].clone(), r["body"]["archived_at"].clone(), r["body"]["cycle_id"].clone()), (Value::Null, Value::Null, Value::Null));
    assert_ne!(r["body"]["created_by"], json!(nobody));
    // a parent that is a work item is told as not supported yet, in Thrust's words; one that is none is as DRF says
    let parent = r["body"]["id"].as_str().unwrap();
    let r = c.call(Caller::ProjectMember, "POST", &issues_path(), with("parent_id", json!(parent)));
    assert_eq!((r["status"].clone(), r["body"].clone()), (json!(400), json!({"parent_id": ["Sub-work items are not supported yet."]})));
    let r = c.call(Caller::ProjectMember, "POST", &issues_path(), with("parent_id", json!(nobody)));
    assert_eq!(r["body"], json!({"parent_id": [format!("Invalid pk \"{nobody}\" - object does not exist.")]}));
    let r = c.call(Caller::ProjectMember, "POST", &issues_path(), with("estimate_point", json!(nobody)));
    assert_eq!(r["body"], json!({"estimate_point": [format!("Invalid pk \"{nobody}\" - object does not exist.")]}));
}

/// Spec: the places of work items run in steps of 10000 from 65535 within a state, as the pin gives them; another state has its
/// own run.
#[test]
fn places_run_in_steps_of_ten_thousand_within_a_state() {
    let c = cast();
    let place = |c: &Cast, body: Value| c.call(Caller::ProjectMember, "POST", &issues_path(), body)["body"]["sort_order"].clone();
    // the cast's project has one state besides the intake one, so a second is added for the test
    seed(
        &c.db,
        SeedFixture {
            states: vec![SeedState {
                id: Uuid::from_u128(0x5003),
                project_id: ProjectId(Uuid::from_u128(PROJECT)),
                workspace_id: WorkspaceId(Uuid::from_u128(WS)),
                name: "Todo".into(),
                color: "#000".into(),
                sequence: 25000.0,
                group: StateGroup::Unstarted,
                is_default: false,
            }],
            ..SeedFixture::default()
        },
    );
    let todo = Uuid::from_u128(0x5003).to_string();
    assert_eq!(place(&c, json!({"name": "a"})), json!(65535.0));
    assert_eq!(place(&c, json!({"name": "b"})), json!(75535.0));
    assert_eq!(place(&c, json!({"name": "c", "state_id": todo})), json!(65535.0), "the first of another state");
    assert_eq!(place(&c, json!({"name": "d"})), json!(85535.0));
    assert_eq!(place(&c, json!({"name": "e", "state_id": todo})), json!(75535.0));
}

/// Spec (Python's `datetime.date` and ISO weeks, which the pin reads dates through): the first and the last year, a year past
/// either, a leap day and a day that is none, a week the year has (2026 has 53, 2025 has 52) and one it has not, weekdays 1 to 7,
/// weeks from 1 on, and digits of the wrong number. Each case was put to the pin (a scratch probe, not a recording): all 19 agree.
#[rstest]
#[case::the_first_year("0001-01-01", true)]
#[case::the_last_year("9999-12-31", true)]
#[case::before_the_first("0000-01-01", false)]
#[case::past_the_last("10000-01-01", false)]
#[case::a_leap_day("2024-02-29", true)]
#[case::not_a_leap_day("2025-02-29", false)]
#[case::a_53rd_week_that_exists("2026-W53-1", true)]
#[case::a_53rd_week_that_does_not("2025-W53-1", false)]
#[case::the_first_week("2026-W01-1", true)]
#[case::week_zero("2026-W00-1", false)]
#[case::weekday_zero("2026-W01-0", false)]
#[case::weekday_seven("2026-W01-7", true)]
#[case::weekday_eight("2026-W01-8", false)]
#[case::a_week_of_one_digit("2026-W1-1", false)]
#[case::a_year_of_three_digits("999-01-01", false)]
#[case::a_month_of_three_digits("2026-001-01", false)]
#[case::eight_digits_that_are_no_day("20261301", false)]
#[case::seven_digits("2026011", false)]
#[case::nine_digits("202601011", false)]
fn a_date_at_the_edge_is_read_as_python_reads_it(#[case] text: &str, #[case] accepted: bool) {
    let c = cast();
    let r = c.call(Caller::ProjectMember, "POST", &issues_path(), with("start_date", json!(text)));
    assert_eq!(r["status"] == 201, accepted, "{text}: {r}");
    if !accepted {
        assert_eq!(r["body"], json!({"start_date": ["Date has wrong format. Use one of these formats instead: YYYY-MM-DD."]}));
    }
}

// --- the list ------------------------------------------------------------------------------------------------------------------

/// Spec (the pin's `get_per_page`, probed on the pin): a page size is a whole number that may have spaces or a plus sign, up to
/// 1000; over the cap, no number (a decimal among them) and (Thrust's own answer, the pin's being a 500) zero and less are 400.
#[rstest]
#[case::one("1", 200, None)]
#[case::the_cap("1000", 200, None)]
#[case::spaces("%202%20", 200, None)]
#[case::a_sign("%2B2", 200, None)]
#[case::over_the_cap("1001", 400, Some("Invalid per_page value. Cannot exceed 1000."))]
#[case::a_decimal("1.0", 400, Some("Invalid per_page parameter."))]
#[case::words("many", 400, Some("Invalid per_page parameter."))]
#[case::zero("0", 400, Some("Invalid per_page parameter."))]
#[case::negative("-1", 400, Some("Invalid per_page parameter."))]
fn a_page_size_is_a_whole_number_from_one_to_the_cap(#[case] size: &str, #[case] status: u64, #[case] message: Option<&str>) {
    let c = cast();
    make(&c, Caller::ProjectMember, json!({"name": "w"}));
    let r = c.call(Caller::ProjectAdmin, "GET", &format!("{}?per_page={size}", issues_path()), Value::Null);
    assert_eq!(r["status"], status, "{size}: {r}");
    if let Some(m) = message {
        assert_eq!(r["body"], json!({"detail": m}));
    }
}

proptest! {
    #![proptest_config(proptest_config(48))]

    /// Spec (the pin's paginator): every key of the envelope is arithmetic over the number of work items, the page size and the
    /// page; the cursors name the page after and before; the rows are the page's.
    #[test]
    fn prop_the_envelope_is_arithmetic_over_the_count_the_size_and_the_page(count in 0..12usize, size in 1..6u32, page in 0..6u32) {
        let c = cast();
        for i in 0..count {
            make(&c, Caller::ProjectMember, json!({"name": format!("w{i}")}));
        }
        let r = c.call(Caller::ProjectAdmin, "GET", &issues_path(), Value::Null);
        prop_assert_eq!(&r["body"]["count"], &json!(count));
        let q = format!("?per_page={size}&cursor=999:{page}:0&order_by=sequence_id");
        let r = c.call(Caller::ProjectAdmin, "GET", &format!("{}{q}", issues_path()), Value::Null);
        let b = &r["body"];
        prop_assert_eq!(&r["status"], &json!(200));
        let (total, rows_before) = (count as u64, u64::from(page) * u64::from(size));
        let on_page = total.saturating_sub(rows_before).min(u64::from(size));
        prop_assert_eq!(&b["count"], &json!(on_page));
        prop_assert_eq!((&b["total_count"], &b["total_results"]), (&json!(total), &json!(total)));
        prop_assert_eq!(&b["total_pages"], &json!(total.div_ceil(u64::from(size))));
        prop_assert_eq!(&b["next_page_results"], &json!(total > rows_before + u64::from(size)));
        prop_assert_eq!(&b["prev_page_results"], &json!(page > 0));
        prop_assert_eq!(&b["next_cursor"], &json!(format!("{size}:{}:0", page + 1)));
        prop_assert_eq!(&b["prev_cursor"], &json!(format!("{size}:{}:1", i64::from(page) - 1)));
        let names: Vec<String> = b["results"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect();
        let want: Vec<String> = (rows_before..rows_before + on_page).map(|i| format!("w{i}")).collect();
        prop_assert_eq!(names, want);
        for key in ["grouped_by", "sub_grouped_by", "extra_stats"] {
            prop_assert_eq!(&b[key], &Value::Null);
        }
    }

    /// Spec (`sanitize_order_by`): an order the pin does not allow, one with two minus signs, one of the three it allows that need a
    /// join (labels, assignees, modules) and an empty one are all `-created_at`.
    #[test]
    fn prop_an_order_that_is_not_offered_is_newest_first(order in prop_oneof![
        Just("name".to_string()), Just("--created_at".to_string()), Just("labels__name".to_string()),
        Just("assignees__first_name".to_string()), Just("issue_module__module__name".to_string()), Just(String::new()),
        "[a-z_]{1,12}".prop_filter("not one of the pin's", |s| !["created_at", "updated_at", "sequence_id", "sort_order", "target_date", "start_date", "completed_at", "archived_at", "priority"].contains(&s.as_str())),
    ]) {
        let c = cast();
        for i in 0..3 {
            make(&c, Caller::ProjectMember, json!({"name": format!("w{i}")}));
        }
        let r = c.call(Caller::ProjectAdmin, "GET", &format!("{}?order_by={order}", issues_path()), Value::Null);
        let names: Vec<&str> = r["body"]["results"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
        prop_assert_eq!(names, vec!["w2", "w1", "w0"], "{}", order);
    }
}

/// Deviation D-028: grouping, and the filters of the pin's 25 and the rich ones that say something, are refused in words of Thrust's
/// own (a flat or an unfiltered list would be wrong data); what the app sends with every request (`sub_issue`, `layout`, an empty
/// `filters`) is not refused, and `filters` that is no JSON object is the pin's own 400.
#[rstest]
#[case::group_by("group_by=priority", 400, json!({"error": "Grouping work items is not supported yet."}))]
#[case::sub_group_by("sub_group_by=state_id&group_by=priority", 400, json!({"error": "Grouping work items is not supported yet."}))]
#[case::a_legacy_filter("priority=high", 400, json!({"error": "Filtering work items is not supported yet."}))]
#[case::another_legacy_filter("state_group=backlog", 400, json!({"error": "Filtering work items is not supported yet."}))]
#[case::a_rich_filter("filters=%7B%22priority__in%22%3A%22high%22%7D", 400, json!({"error": "Filtering work items is not supported yet."}))]
#[case::filters_that_are_no_json("filters=nonsense", 400, json!({"message": "Invalid JSON for 'filter'. Expected a valid JSON object.", "code": "invalid_json"}))]
#[case::the_apps_own_parameters("sub_issue=false&layout=list&filters=%7B%7D&order_by=-created_at", 200, Value::Null)]
#[case::empty_values("priority=&group_by=&filters=", 200, Value::Null)]
fn what_a_list_does_not_answer_is_refused_in_its_own_words(#[case] query: &str, #[case] status: u64, #[case] body: Value) {
    let c = cast();
    let r = c.call(Caller::ProjectAdmin, "GET", &format!("{}?{query}", issues_path()), Value::Null);
    assert_eq!(r["status"], status, "{r}");
    if status != 200 {
        assert_eq!(r["body"], body);
    }
}

/// Spec (`updated_at__gt`): what changed after a time. A time that is no time is the pin's 400; a date is midnight.
#[test]
fn updated_at_gt_keeps_what_changed_after_it() {
    let c = cast();
    make(&c, Caller::ProjectMember, json!({"name": "w"}));
    let count = |q: &str| c.call(Caller::ProjectAdmin, "GET", &format!("{}?{q}", issues_path()), Value::Null);
    // the work item was made at the world's fixed instant, in October 2025
    assert_eq!(count("updated_at__gt=2099-01-01T00:00:00Z")["body"]["count"], 0);
    assert_eq!(count("updated_at__gt=2099-01-01")["body"]["count"], 0);
    assert_eq!(count("updated_at__gt=2001-01-01T00:00:00Z")["body"]["count"], 1);
    assert_eq!(count("updated_at__gt=2001-01-01%2000:00:00")["body"]["count"], 1);
    let bad = count("updated_at__gt=nonsense");
    assert_eq!((bad["status"].clone(), bad["body"].clone()), (json!(400), json!({"error": "Please provide valid detail"})));
}

// --- opening one ---------------------------------------------------------------------------------------------------------------

/// Spec (`expand`): reactions, attachments, links and the parent are added when named, as the nothing they are here; a name the pin
/// does not know adds nothing.
#[test]
fn what_the_app_asks_to_expand_is_added_as_nothing() {
    let c = cast();
    let id = make(&c, Caller::ProjectMember, json!({"name": "w"}));
    let plain = c.call(Caller::ProjectMember, "GET", &issue_path(&id), Value::Null)["body"].clone();
    for key in ["issue_reactions", "issue_attachments", "issue_link", "parent"] {
        assert!(plain.get(key).is_none(), "{key}");
    }
    let expanded = c.call(Caller::ProjectMember, "GET", &format!("{}?expand=issue_reactions,issue_attachments,issue_link,parent,nonsense", issue_path(&id)), Value::Null)["body"].clone();
    for key in ["issue_reactions", "issue_attachments", "issue_link"] {
        assert_eq!(expanded[key], json!([]), "{key}");
    }
    assert_eq!(expanded["parent"], Value::Null);
    assert!(expanded.get("nonsense").is_none());
    let mut without = expanded.as_object().unwrap().clone();
    for key in ["issue_reactions", "issue_attachments", "issue_link", "parent"] {
        without.remove(key);
    }
    assert_eq!(Value::Object(without), plain, "the expansion adds keys and changes none");
    // and the detail has the description and the subscription that the list's rows lack, and no state group
    assert!(plain.get("description_html").is_some() && plain.get("is_subscribed").is_some() && plain.get("state__group").is_none());
}

/// Spec: an id in the path that is no id is Django's 404 for a path no route has; one that is an id and no work item is the view's.
#[test]
fn an_id_that_is_no_work_item_is_a_404() {
    let c = cast();
    let not_an_id = c.call(Caller::ProjectMember, "GET", &issue_path("nope"), Value::Null);
    assert_eq!((not_an_id["status"].clone(), not_an_id["body"].clone()), (json!(404), json!({"error": "Page not found."})));
    let missing = c.call(Caller::ProjectMember, "GET", &issue_path("00000000-0000-4000-8000-000000000bad"), Value::Null);
    assert_eq!((missing["status"].clone(), missing["body"].clone()), (json!(404), json!({"error": "The required object does not exist."})));
}
