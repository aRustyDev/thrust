//! What the routes that change a work item (X03), read its history (X02) and answer what its page asks besides (the sub work items,
//! the relations) derive from callers, bodies and rows that the recorded journey never has: a project guest who made the work item, a
//! request that names its fields in an order, a `skip_activity` of every kind, a place in the list, a time that is no time. The pin's
//! answers for those come from its source and from probes, so these tests state the rules instead of replaying a recording.
//!
//! What must hold: no session is 401 on every row; whoever is not in the project is refused (the decorator's 403 for a change, the
//! permission class's for the reads); a change is read as DRF reads a partial update, each field it names in its own words and the
//! ones it does not name left alone; a change records one row for each difference in the order the request names them, and none when
//! `skip_activity` is true of a description; the history is read after a time strictly; and what Thrust does not do that the pin does
//! (deviation D-029) it refuses or ignores as written down.

mod common;

use common::*;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{json, Value};
use thrust_contracts::*;
use thrust_ports::*;

const TODO: u128 = 0x5003;
const DONE: u128 = 0x5004;

const UNAUTHENTICATED: &str = "Authentication credentials were not provided.";

fn by_decorator() -> Value {
    json!({"error": "You don't have the required permissions."})
}

fn by_class() -> Value {
    json!({"detail": "You do not have permission to perform this action."})
}

fn issues_path() -> String {
    project_path("issues/")
}

fn issue_path(id: &str) -> String {
    project_path(&format!("issues/{id}/"))
}

fn history_path(id: &str) -> String {
    project_path(&format!("issues/{id}/history/"))
}

/// The cast, with a state of the unstarted group and one of the completed group besides the default one.
fn cast_with_states() -> Cast {
    let c = cast();
    let state = |n: u128, name: &str, sequence: f64, group: StateGroup| SeedState {
        id: Uuid::from_u128(n),
        project_id: ProjectId(Uuid::from_u128(PROJECT)),
        workspace_id: WorkspaceId(Uuid::from_u128(WS)),
        name: name.into(),
        color: "#000000".into(),
        sequence,
        group,
        is_default: false,
    };
    seed(
        &c.db,
        SeedFixture {
            states: vec![
                state(TODO, "Todo", 20_000.0, StateGroup::Unstarted),
                state(DONE, "Done", 30_000.0, StateGroup::Completed),
            ],
            ..SeedFixture::default()
        },
    );
    c
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
        created_by: Some(c.user(who)),
        created_at: at(1),
        archived_at: None,
        is_draft: false,
        deleted_at: None,
    };
    futures_executor::block_on(tx.insert_work_item(&scope, &item)).unwrap();
    futures_executor::block_on(tx.commit()).unwrap();
    id.0.to_string()
}

fn patch(c: &Cast, who: Caller, id: &str, body: Value) -> Value {
    c.call(who, "PATCH", &issue_path(id), body)
}

fn detail(c: &Cast, id: &str) -> Value {
    let r = c.call(Caller::ProjectMember, "GET", &issue_path(id), Value::Null);
    assert_eq!(r["status"], 200, "{r}");
    r["body"].clone()
}

/// The rows of a work item's history, oldest first.
fn rows(c: &Cast, id: &str) -> Vec<Value> {
    let r = c.call(
        Caller::ProjectMember,
        "GET",
        &format!("{}?activity_type=issue-property", history_path(id)),
        Value::Null,
    );
    assert_eq!(r["status"], 200, "{r}");
    r["body"].as_array().unwrap().clone()
}

fn fields_of(rows: &[Value]) -> Vec<String> {
    rows.iter().map(|r| r["field"].as_str().map_or("(creation)".to_string(), str::to_string)).collect()
}

// --- who may ask ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    X03,
    X02,
    SubIssues,
    Relations,
}

/// Spec (the pin's `allow_permission(creator=True)` on the change, `ProjectEntityPermission` on the others): no session is 401
/// everywhere; a change is made by an admin or a member of the project, and by nobody else (a guest who did not make the work item
/// is refused with the decorator's words, so is a workspace's admin who is not in the project); the three reads are open to every
/// member of the project, a guest too, and the rest are refused in DRF's own words.
#[rstest]
fn each_row_answers_each_kind_of_caller_as_the_pins_guard_does(
    #[values(Row::X03, Row::X02, Row::SubIssues, Row::Relations)] row: Row,
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
    let id = make(&c, Caller::ProjectAdmin, json!({"name": "made by the admin"}));
    let r = match row {
        Row::X03 => patch(&c, who, &id, json!({"name": "changed"})),
        Row::X02 => c.call(who, "GET", &history_path(&id), Value::Null),
        Row::SubIssues => c.call(who, "GET", &format!("{}sub-issues/", issue_path(&id)), Value::Null),
        Row::Relations => c.call(who, "GET", &format!("{}issue-relation/", issue_path(&id)), Value::Null),
    };
    let (status, body) = (r["status"].as_u64().unwrap(), r["body"].clone());
    match (who, row) {
        (Caller::Anonymous, _) => assert_eq!((status, body), (401, json!({"detail": UNAUTHENTICATED}))),
        (Caller::ProjectAdmin | Caller::ProjectMember, Row::X03) => assert_eq!((status, body), (204, Value::Null)),
        (_, Row::X03) => assert_eq!((status, body), (403, by_decorator()), "{who:?}"),
        (who, _) if who.in_project() => assert_eq!(status, 200, "{who:?} {row:?}: {body}"),
        _ => assert_eq!((status, body), (403, by_class()), "{who:?} {row:?}"),
    }
}

/// Spec (`creator=True`): whoever made a work item changes it, whatever their role in the project (a member who is made a guest keeps
/// their own work); a guest does not change another's. The change is in the history under their name. (A creator who has left the
/// project is not let in; deviation D-029.)
#[test]
fn a_guest_changes_the_work_item_they_made_and_the_history_says_who() {
    let c = cast();
    let own = seed_item_by(&c, Caller::ProjectGuest, "the guest's");
    let others = make(&c, Caller::ProjectMember, json!({"name": "a member's"}));
    assert_eq!(patch(&c, Caller::ProjectGuest, &own, json!({"name": "renamed by its maker"}))["status"], 204);
    let refused = patch(&c, Caller::ProjectGuest, &others, json!({"name": "renamed by a guest"}));
    assert_eq!((refused["status"].clone(), refused["body"].clone()), (json!(403), by_decorator()));
    assert_eq!(detail(&c, &own)["name"], "renamed by its maker");
    assert_eq!(detail(&c, &others)["name"], "a member's");
    let history = rows(&c, &own);
    assert_eq!(fields_of(&history), vec!["name"]);
    assert_eq!(history[0]["actor"], json!(c.user(Caller::ProjectGuest).0));
    assert_eq!(history[0]["actor_detail"]["id"], json!(c.user(Caller::ProjectGuest).0));
    // a guest who is not the maker, and a work item that is not there, are the decorator's 403 too (it has nothing to say they made)
    let gone = patch(&c, Caller::ProjectGuest, "00000000-0000-4000-8000-000000000bad", json!({}));
    assert_eq!((gone["status"].clone(), gone["body"].clone()), (json!(403), by_decorator()));
}

/// Spec: the work item is looked for before the body is read (the pin's view fetches it first): a work item that is not there is 404
/// whatever the body is, a body that is no object is 400 only for one that is there, and an id that is no id is a path no route has.
#[test]
fn a_work_item_that_is_not_there_is_404_before_the_body_is_read() {
    let c = cast();
    let id = make(&c, Caller::ProjectMember, json!({"name": "n"}));
    let gone = "00000000-0000-4000-8000-000000000bad";
    let bad_body = c.call_raw(Caller::ProjectMember, "PATCH", &issue_path(gone), "[1, 2");
    assert_eq!(
        (bad_body["status"].clone(), bad_body["body"].clone()),
        (json!(404), json!({"error": "Issue not found"}))
    );
    let invalid_field = patch(&c, Caller::ProjectMember, gone, json!({"name": ""}));
    assert_eq!(
        (invalid_field["status"].clone(), invalid_field["body"].clone()),
        (json!(404), json!({"error": "Issue not found"}))
    );
    let there = c.call_raw(Caller::ProjectMember, "PATCH", &issue_path(&id), "[1, 2");
    assert_eq!(there["status"], 400, "{there}");
    let not_an_id = patch(&c, Caller::ProjectMember, "nope", json!({}));
    assert_eq!(
        (not_an_id["status"].clone(), not_an_id["body"].clone()),
        (json!(404), json!({"error": "Page not found."}))
    );
}

// --- reading a change the way DRF does -----------------------------------------------------------------------------------------

const DATE_MESSAGE: &str = "Date has wrong format. Use one of these formats instead: YYYY-MM-DD.";

/// A field of a partial update: its name, a value that is valid and changes the work item, one that is not, and DRF's message for it.
fn fields(c: &Cast) -> Vec<(&'static str, Value, Value, Value)> {
    let member = c.user(Caller::ProjectMember).0.to_string();
    vec![
        ("name", json!("Changed"), json!(""), json!(["This field may not be blank."])),
        ("priority", json!("high"), json!("huge"), json!(["\"huge\" is not a valid choice."])),
        ("state_id", json!(Uuid::from_u128(TODO)), json!("nope"), json!(["\u{201c}nope\u{201d} is not a valid UUID."])),
        ("start_date", json!("2026-11-01"), json!("yesterday"), json!([DATE_MESSAGE])),
        ("target_date", json!("2026-11-15"), json!("2026-13-45"), json!([DATE_MESSAGE])),
        ("description_html", json!("<p>new</p>"), Value::Null, json!(["This field may not be null."])),
        ("assignee_ids", json!([member]), json!("x"), json!(["Expected a list of items but got type \"str\"."])),
        ("label_ids", json!([]), Value::Null, json!(["This field may not be null."])),
    ]
}

proptest! {
    #![proptest_config(proptest_config(64))]

    /// Spec (DRF's `partial=True`): only the fields a request names are checked, each in its own words, all at once; a request with
    /// any wrong one is 400 and changes nothing, and one with none wrong answers 204 and changes exactly the fields it names.
    #[test]
    fn prop_a_partial_update_checks_only_what_it_names(chosen in prop::collection::vec((0..8usize, any::<bool>()), 0..9)) {
        let c = cast_with_states();
        let id = make(&c, Caller::ProjectMember, json!({"name": "n"}));
        let before = detail(&c, &id);
        let table = fields(&c);
        let mut named: Vec<(usize, bool)> = Vec::new();
        for (i, valid) in chosen {
            if !named.iter().any(|(j, _)| *j == i) {
                named.push((i, valid));
            }
        }
        let mut body = serde_json::Map::new();
        let mut wrong = serde_json::Map::new();
        for (i, valid) in &named {
            let (field, good, bad, message) = &table[*i];
            body.insert((*field).to_string(), if *valid { good.clone() } else { bad.clone() });
            if !valid {
                wrong.insert((*field).to_string(), message.clone());
            }
        }
        let r = patch(&c, Caller::ProjectMember, &id, Value::Object(body));
        let after = detail(&c, &id);
        if !wrong.is_empty() {
            prop_assert_eq!((&r["status"], &r["body"]), (&json!(400), &Value::Object(wrong)));
            prop_assert_eq!(&after, &before, "a refused change changes nothing");
        } else {
            prop_assert_eq!((&r["status"], &r["body"]), (&json!(204), &Value::Null));
            for (i, _) in &named {
                let (field, good, _, _) = &table[*i];
                prop_assert_eq!(&after[*field], good, "{}", field);
            }
            // and what it did not name is as it was
            for (i, (field, ..)) in table.iter().enumerate() {
                if !named.iter().any(|(j, _)| *j == i) {
                    prop_assert_eq!(&after[*field], &before[*field], "{}", field);
                }
            }
            prop_assert_eq!(&after["updated_by"], &json!(c.user(Caller::ProjectMember).0));
        }
    }

    /// Spec (`update_issue_activity` walks the request's keys in the order the body has them): a request with several fields has its
    /// changes in the history in the order it names them, whatever the order is; each is a row of its own.
    #[test]
    fn prop_the_history_lists_a_requests_changes_in_the_order_it_names_them(order in Just((0..6usize).collect::<Vec<_>>()).prop_shuffle()) {
        let c = cast_with_states();
        let id = make(&c, Caller::ProjectMember, json!({"name": "n"}));
        let members = [
            ("name", "\"Changed\"".to_string(), "name"),
            ("priority", "\"high\"".to_string(), "priority"),
            ("description_html", "\"<p>new</p>\"".to_string(), "description"),
            ("start_date", "\"2026-11-01\"".to_string(), "start_date"),
            ("target_date", "\"2026-11-15\"".to_string(), "target_date"),
            ("state_id", format!("\"{}\"", Uuid::from_u128(TODO)), "state"),
        ];
        let raw = format!("{{{}}}", order.iter().map(|i| format!("\"{}\": {}", members[*i].0, members[*i].1)).collect::<Vec<_>>().join(", "));
        let r = c.call_raw(Caller::ProjectMember, "PATCH", &issue_path(&id), &raw);
        prop_assert_eq!(&r["status"], &json!(204), "{} {}", raw, r);
        let history = rows(&c, &id);
        let want: Vec<String> = std::iter::once("(creation)".to_string()).chain(order.iter().map(|i| members[*i].2.to_string())).collect();
        prop_assert_eq!(fields_of(&history), want, "{}", raw);
    }
}

/// Spec (Python's `if skip_activity`): a description written with a `skip_activity` that is true leaves no row (and so no webhook),
/// one that is false does; the text `"false"` and any non-empty list are true; `skip_activity` means nothing for a change that is
/// not a description. The change is made either way.
#[rstest]
fn skip_activity_is_python_truth_and_applies_to_a_description(
    #[values(json!(true), json!(false), json!("false"), json!("x"), json!(""), json!(0), json!(1), json!(0.0), json!(null), json!([]), json!([0]), json!({}), json!({"a": 1}))]
    skip: Value,
) {
    let truthy = match &skip {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    };
    let c = cast();
    let id = make(&c, Caller::ProjectMember, json!({"name": "n"}));
    assert_eq!(
        patch(&c, Caller::ProjectMember, &id, json!({"skip_activity": skip, "description_html": "<p>d</p>"}))["status"],
        204
    );
    assert_eq!(detail(&c, &id)["description_html"], "<p>d</p>", "the description is written either way");
    assert_eq!(
        fields_of(&rows(&c, &id)),
        if truthy { vec!["(creation)"] } else { vec!["(creation)", "description"] },
        "{skip}"
    );
    assert_eq!(patch(&c, Caller::ProjectMember, &id, json!({"skip_activity": skip, "name": "m"}))["status"], 204);
    let history = fields_of(&rows(&c, &id));
    assert_eq!(
        history.last().map(String::as_str),
        Some("name"),
        "a name is recorded whatever skip_activity is: {skip}"
    );
}

// --- what Thrust does not do that the pin does (D-029) --------------------------------------------------------------------------

/// Spec (deviation D-029): the start must not be after the target counting the date the work item has when the request names one
/// only; the pin accepts that and stores the inverted pair. The same day, and a date when the work item has none of the other, are
/// fine.
#[test]
fn a_start_after_the_target_the_work_item_has_is_refused() {
    let c = cast();
    let id = make(&c, Caller::ProjectMember, json!({"name": "n", "target_date": "2026-11-15"}));
    let r = patch(&c, Caller::ProjectMember, &id, json!({"start_date": "2026-12-31"}));
    assert_eq!(
        (r["status"].clone(), r["body"].clone()),
        (json!(400), json!({"non_field_errors": ["Start date cannot exceed target date"]}))
    );
    assert_eq!(detail(&c, &id)["start_date"], Value::Null);
    assert_eq!(patch(&c, Caller::ProjectMember, &id, json!({"start_date": "2026-11-15"}))["status"], 204);
    // moving the target before the start is the same refusal, and clearing the start lets it go
    let r = patch(&c, Caller::ProjectMember, &id, json!({"target_date": "2026-11-01"}));
    assert_eq!(
        (r["status"].clone(), r["body"].clone()),
        (json!(400), json!({"non_field_errors": ["Start date cannot exceed target date"]}))
    );
    assert_eq!(
        patch(&c, Caller::ProjectMember, &id, json!({"start_date": null, "target_date": "2026-11-01"}))["status"],
        204
    );
}

/// Spec (deviation D-029): a place in the list is not supported yet, so a request that names one is refused in Thrust's words
/// (null and what is no number in DRF's) where the pin moves the work item; the work item stays where it was.
#[rstest]
fn a_place_in_the_list_is_refused(
    #[values(json!(5), json!(5.5), json!("5"), json!(" 7 "), json!(null), json!("x"), json!(true), json!([]))]
    place: Value,
) {
    let c = cast();
    let id = make(&c, Caller::ProjectMember, json!({"name": "n"}));
    let before = detail(&c, &id);
    let message = match &place {
        Value::Null => "This field may not be null.",
        Value::Number(_) => "Moving work items is not supported yet.",
        Value::String(s) if s.trim().parse::<f64>().is_ok() => "Moving work items is not supported yet.",
        _ => "A valid number is required.",
    };
    let r = patch(&c, Caller::ProjectMember, &id, json!({"sort_order": place, "name": "also"}));
    assert_eq!((r["status"].clone(), r["body"].clone()), (json!(400), json!({"sort_order": [message]})), "{place}");
    assert_eq!(detail(&c, &id), before);
}

/// Spec (deviation D-029): what the pin's serializer accepts but no app sends (the sequence number, the legacy point, the external
/// source and id, a draft, an archive, the model's own `state` and `parent`, the automation's `closed_to`) is ignored: 204, and the
/// work item is as it was but for when and by whom it was changed. Nothing is recorded of them.
#[test]
fn what_the_pin_accepts_but_no_app_sends_is_ignored() {
    let c = cast_with_states();
    let id = make(&c, Caller::ProjectMember, json!({"name": "n"}));
    let before = detail(&c, &id);
    let nobody = "00000000-0000-4000-8000-00000000dead";
    let r = patch(
        &c,
        Caller::ProjectMember,
        &id,
        json!({
            "sequence_id": 99, "point": 3, "external_source": "s", "external_id": "e", "is_draft": true, "archived_at": "2026-01-01",
            "state": Uuid::from_u128(TODO), "parent": nobody, "closed_to": Uuid::from_u128(DONE), "labels": [nobody], "assignees": [nobody],
            "id": nobody, "created_by": nobody, "completed_at": "2026-01-01T00:00:00Z", "cycle_id": nobody, "module_ids": [nobody], "type_id": nobody,
        }),
    );
    assert_eq!((r["status"].clone(), r["body"].clone()), (json!(204), Value::Null));
    let after = detail(&c, &id);
    let strip = |mut v: Value| {
        for k in ["updated_at", "updated_by"] {
            v.as_object_mut().unwrap().remove(k);
        }
        v
    };
    assert_eq!(strip(after), strip(before));
    assert_eq!(fields_of(&rows(&c, &id)), vec!["(creation)"]);
}

/// Spec (deviation D-029): what the history keeps of a description is what was stored, cleaned of what can run, not what was sent
/// (the pin keeps the markup it cleaned away).
#[test]
fn the_history_keeps_a_description_as_it_was_cleaned() {
    let c = cast();
    let id = make(&c, Caller::ProjectMember, json!({"name": "n"}));
    assert_eq!(
        patch(
            &c,
            Caller::ProjectMember,
            &id,
            json!({"description_html": "<p onclick=\"x()\">y</p><script>alert(1)</script>"})
        )["status"],
        204
    );
    let stored = detail(&c, &id)["description_html"].as_str().unwrap().to_string();
    assert!(!stored.contains("script") && !stored.contains("onclick"), "{stored}");
    let history = rows(&c, &id);
    assert_eq!(history[1]["field"], "description");
    assert_eq!(history[1]["new_value"], json!(stored));
    assert_eq!(history[1]["old_value"], "<p></p>");
}

/// Spec (D-028, as for a creation): a parent that is one of the project's work items is refused as not supported yet, one that is
/// none is as DRF says, and null is no parent (nothing to say); an estimate takes null only.
#[test]
fn a_parent_and_an_estimate_are_refused_as_for_a_creation() {
    let c = cast();
    let id = make(&c, Caller::ProjectMember, json!({"name": "n"}));
    let other = make(&c, Caller::ProjectMember, json!({"name": "other"}));
    let nobody = "00000000-0000-4000-8000-00000000dead";
    let body = |v: Value| {
        let r = patch(&c, Caller::ProjectMember, &id, v);
        (r["status"].clone(), r["body"].clone())
    };
    assert_eq!(
        body(json!({"parent_id": other})),
        (json!(400), json!({"parent_id": ["Sub-work items are not supported yet."]}))
    );
    assert_eq!(
        body(json!({"parent_id": nobody})),
        (json!(400), json!({"parent_id": [format!("Invalid pk \"{nobody}\" - object does not exist.")]}))
    );
    assert_eq!(body(json!({"parent_id": null, "estimate_point": null})), (json!(204), Value::Null));
    assert_eq!(
        body(json!({"estimate_point": nobody})),
        (json!(400), json!({"estimate_point": [format!("Invalid pk \"{nobody}\" - object does not exist.")]}))
    );
    assert_eq!(detail(&c, &id)["parent_id"], Value::Null);
}

// --- the history ---------------------------------------------------------------------------------------------------------------

/// Spec (`created_at__gt`): the rows made strictly after a time, the time read as Django reads a datetime (RFC 3339, with a space for
/// the `T` and no zone, or a date at midnight); one that is none, an empty one included, is 400, for the comments as for the changes.
#[rstest]
fn the_history_is_read_after_a_time_strictly(
    #[values(
        ("2025-10-09T08:53:19.999999Z", 2),
        ("2025-10-09T08:53:20Z", 0),
        ("2025-10-09%2008:53:19", 2),
        ("2025-10-09T08:53:20.000001Z", 0),
        ("2025-10-09", 2),
        ("2025-10-10", 0),
        ("2001-01-01", 2),
        ("2099-01-01T00:00:00%2B02:00", 0)
    )]
    case: (&str, usize),
) {
    let (after, count) = case;
    let c = cast();
    let id = make(&c, Caller::ProjectMember, json!({"name": "n"}));
    assert_eq!(patch(&c, Caller::ProjectMember, &id, json!({"name": "m"}))["status"], 204);
    let r = c.call(
        Caller::ProjectMember,
        "GET",
        &format!("{}?activity_type=issue-property&created_at__gt={after}", history_path(&id)),
        Value::Null,
    );
    assert_eq!((r["status"].clone(), r["body"].as_array().map(Vec::len)), (json!(200), Some(count)), "{after}");
}

#[rstest]
fn a_time_that_is_none_is_400_for_either_kind_of_history(
    #[values("nonsense", "", "2025-13-01", "yesterday")] after: &str,
    #[values("issue-property", "issue-comment", "")] kind: &str,
) {
    let c = cast();
    let id = make(&c, Caller::ProjectMember, json!({"name": "n"}));
    let query = if kind.is_empty() {
        format!("created_at__gt={after}")
    } else {
        format!("activity_type={kind}&created_at__gt={after}")
    };
    let r = c.call(Caller::ProjectMember, "GET", &format!("{}?{query}", history_path(&id)), Value::Null);
    assert_eq!(
        (r["status"].clone(), r["body"].clone()),
        (json!(400), json!({"error": "Please provide valid detail"})),
        "{query}"
    );
}

/// Spec: the comments of a work item are none (nothing makes one); every kind but `issue-comment` is the changes, so is no kind;
/// a work item that is not there has an empty history, not a 404.
#[rstest]
fn the_kinds_of_history(
    #[values("issue-property", "issue-comment", "epic-comment", "epic-property", "bogus", "")] kind: &str,
) {
    let c = cast();
    let id = make(&c, Caller::ProjectMember, json!({"name": "n"}));
    let query = if kind.is_empty() { String::new() } else { format!("?activity_type={kind}") };
    let r = c.call(Caller::ProjectMember, "GET", &format!("{}{query}", history_path(&id)), Value::Null);
    assert_eq!(r["status"], 200);
    assert_eq!(r["body"].as_array().unwrap().len(), usize::from(kind != "issue-comment"), "{kind}");
    let gone = c.call(
        Caller::ProjectMember,
        "GET",
        &format!("{}{query}", history_path("00000000-0000-4000-8000-000000000bad")),
        Value::Null,
    );
    assert_eq!((gone["status"].clone(), gone["body"].clone()), (json!(200), json!([])));
}

/// Spec (`IssueActivitySerializer`): a row of the history is the model's twenty-odd columns and four details, with the work item as
/// it is now and the people as they are called now; `epoch` is a number, the creation has no field, and no row has a comment
/// reaction, an attachment or an intake.
#[test]
fn a_row_of_the_history_has_the_pins_keys() {
    let c = cast();
    let id = make(&c, Caller::ProjectMember, json!({"name": "n"}));
    assert_eq!(patch(&c, Caller::ProjectMember, &id, json!({"name": "m"}))["status"], 204);
    let history = rows(&c, &id);
    assert_eq!(history.len(), 2);
    // the set of keys, not their order: `serde_json` keeps the order of the text when another crate of the workspace asks for
    // `preserve_order` (the editor's), and sorts them otherwise
    let keys = |v: &Value| {
        let mut keys = v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        keys.sort();
        keys
    };
    let want: Vec<&str> = [
        "actor",
        "actor_detail",
        "attachments",
        "comment",
        "created_at",
        "created_by",
        "deleted_at",
        "epoch",
        "field",
        "id",
        "issue",
        "issue_comment",
        "issue_detail",
        "new_identifier",
        "new_value",
        "old_identifier",
        "old_value",
        "project",
        "project_detail",
        "source_data",
        "updated_at",
        "updated_by",
        "verb",
        "workspace",
        "workspace_detail",
    ]
    .to_vec();
    for row in &history {
        assert_eq!(keys(row), want);
        assert!(row["epoch"].is_f64(), "{}", row["epoch"]);
        assert_eq!(
            keys(&row["actor_detail"]),
            vec!["avatar", "avatar_url", "display_name", "first_name", "id", "is_bot", "last_name"]
        );
        assert_eq!(
            keys(&row["issue_detail"]),
            vec![
                "description_html",
                "description_json",
                "id",
                "is_draft",
                "name",
                "priority",
                "sequence_id",
                "sort_order",
                "start_date",
                "target_date"
            ]
        );
        assert_eq!(
            keys(&row["project_detail"]),
            vec!["cover_image", "cover_image_url", "description", "id", "identifier", "logo_props", "name"]
        );
        assert_eq!(keys(&row["workspace_detail"]), vec!["id", "logo_url", "name", "slug"]);
        // the work item as it is now, in every row
        assert_eq!(row["issue_detail"]["name"], "m");
    }
    assert_eq!(
        (history[0]["verb"].clone(), history[0]["field"].clone(), history[0]["comment"].clone()),
        (json!("created"), Value::Null, json!("created the issue"))
    );
    assert_eq!(
        (
            history[1]["verb"].clone(),
            history[1]["field"].clone(),
            history[1]["old_value"].clone(),
            history[1]["new_value"].clone()
        ),
        (json!("updated"), json!("name"), json!("n"), json!("m"))
    );
}

// --- what the work item's page asks besides ----------------------------------------------------------------------------------------

/// Spec (D-021, stubs): a work item has no sub work items and no relations, so they are the pin's empty answers. The pin does not look
/// for the work item for the sub work items, and does for the relations (404 with its own words); an id that is no id is no route.
#[test]
fn the_sub_work_items_and_the_relations_of_a_work_item_are_none() {
    let c = cast();
    let id = make(&c, Caller::ProjectMember, json!({"name": "n"}));
    let gone = "00000000-0000-4000-8000-000000000bad";
    let get = |path: String, who: Caller| {
        let r = c.call(who, "GET", &path, Value::Null);
        (r["status"].clone(), r["body"].clone())
    };
    let sub = |id: &str| format!("{}sub-issues/", issue_path(id));
    let relation = |id: &str| format!("{}issue-relation/", issue_path(id));
    let empty_sub = (json!(200), json!({"sub_issues": [], "state_distribution": {}}));
    assert_eq!(get(sub(&id), Caller::ProjectGuest), empty_sub);
    assert_eq!(get(sub(gone), Caller::ProjectMember), empty_sub);
    assert_eq!(get(sub("nope"), Caller::ProjectMember), (json!(404), json!({"error": "Page not found."})));
    let empty_relations = json!({"blocking": [], "blocked_by": [], "duplicate": [], "relates_to": [], "start_after": [], "start_before": [], "finish_after": [], "finish_before": []});
    assert_eq!(get(relation(&id), Caller::ProjectGuest), (json!(200), empty_relations));
    assert_eq!(get(relation(gone), Caller::ProjectMember), (json!(404), json!({"error": "Issue not found"})));
    assert_eq!(get(relation("nope"), Caller::ProjectMember), (json!(404), json!({"error": "Page not found."})));
}
