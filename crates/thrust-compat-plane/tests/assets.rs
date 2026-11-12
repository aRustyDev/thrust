//! The asset stage's routes (S28, S29, S31 and S33, and the three around them that only Thrust has) over what the recorded journey
//! never has: every kind of caller, an upload that is wrong in each way it can be, an asset that belongs to a project, and a
//! project made or changed with a cover. The pin's refusals come from its source (`app/views/asset/v2.py`, `settings/storage.py`)
//! and, for the cases a journey can record, from the fixtures of `parity.rs`; these tests state the rules.
//!
//! What must hold: no session is a 401 on every route that asks who the caller is; a stranger to the workspace is refused by the
//! decorator; an asset's upload is made once, to the address S28 gave, with the type and the size it was made for; a project's
//! asset is read only by the project's people; a cover reaches a project through S31 (or S30 and S32's `cover_image_asset`), and
//! what the app reads of the project says so.

mod common;

use common::*;
use serde_json::{json, Value};
use thrust_contracts::Uuid;

const PNG: &str = "image/png";
const LIMIT: usize = 5 * 1024 * 1024;

fn assets_path(tail: &str) -> String {
    format!("/api/assets/v2/workspaces/{SLUG}/{tail}")
}

fn cover_body(name: &str, content_type: &str, size: usize) -> Value {
    json!({"name": name, "type": content_type, "size": size, "entity_type": "PROJECT_COVER", "entity_identifier": ""})
}

/// S28 as `who`.
fn s28(c: &Cast, who: Caller, body: Value) -> Value {
    c.call(who, "POST", &assets_path(""), body)
}

/// A made asset: its id and the address its bytes go to.
struct Made {
    id: String,
    url: String,
}

fn made(c: &Cast, who: Caller, name: &str, size: usize) -> Made {
    let r = s28(c, who, cover_body(name, PNG, size));
    assert_eq!(r["status"], 200, "{r}");
    let b = &r["body"];
    Made { id: b["asset_id"].as_str().unwrap().to_string(), url: b["upload_data"]["url"].as_str().unwrap().to_string() }
}

/// The browser's upload: no session, the type and the bytes.
fn put(c: &Cast, url: &str, content_type: &str, bytes: Vec<u8>) -> Raw {
    raw(&c.router, &c.jar, "", "PUT", url, &[("content-type", content_type)], bytes)
}

fn get(c: &Cast, who: Caller, path: &str) -> Raw {
    raw(&c.router, &c.jar, who.email(), "GET", path, &[], vec![])
}

/// An asset that is made, uploaded and confirmed, as `who`.
fn uploaded(c: &Cast, who: Caller, name: &str, bytes: Vec<u8>) -> String {
    let m = made(c, who, name, bytes.len());
    assert_eq!(put(c, &m.url, PNG, bytes).status, 200);
    let r = c.call(who, "PATCH", &assets_path(&format!("{}/", m.id)), Value::Null);
    assert_eq!(r["status"], 204, "{r}");
    m.id
}

fn bulk(c: &Cast, who: Caller, ids: Value) -> Value {
    let path = format!(
        "/api/assets/v2/workspaces/{SLUG}/projects/{}/{}/bulk/",
        Uuid::from_u128(PROJECT),
        Uuid::from_u128(PROJECT)
    );
    c.call(who, "POST", &path, json!({"asset_ids": ids}))
}

fn unauthenticated() -> Value {
    json!({"detail": "Authentication credentials were not provided."})
}

fn by_decorator() -> Value {
    json!({"error": "You don't have the required permissions."})
}

// --- S28 ---------------------------------------------------------------------------------------------------------------------

#[test]
fn s28_is_for_every_member_of_the_workspace_and_the_answer_is_the_pins_presigned_put() {
    let c = cast();
    for who in CALLERS {
        let r = s28(&c, who, cover_body("cover.png", PNG, 70));
        match who {
            Caller::Anonymous => {
                assert_eq!((r["status"].as_u64(), &r["body"]), (Some(401), &unauthenticated()), "{who:?}")
            }
            Caller::Stranger => assert_eq!((r["status"].as_u64(), &r["body"]), (Some(403), &by_decorator()), "{who:?}"),
            _ => {
                assert_eq!(r["status"], 200, "{who:?}: {r}");
                let b = &r["body"];
                let id = b["asset_id"].as_str().unwrap();
                assert_eq!(b["asset_url"], format!("/api/assets/v2/static/{id}/"));
                let d = &b["upload_data"];
                assert_eq!((&d["method"], &d["fields"]), (&json!("PUT"), &json!({})));
                assert_eq!(d["headers"], json!({"Content-Type": "image/png", "Content-Length": "70"}));
                let url = d["url"].as_str().unwrap();
                let token = url
                    .strip_prefix(&format!("/api/assets/v2/upload/{id}/"))
                    .and_then(|t| t.strip_suffix('/'))
                    .unwrap();
                assert!(token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()), "{token}");
                let mut keys: Vec<&str> = b.as_object().unwrap().keys().map(String::as_str).collect();
                keys.sort_unstable();
                assert_eq!(keys, ["asset_id", "asset_url", "upload_data"]);
            }
        }
    }
}

#[test]
fn two_assets_get_two_tokens() {
    let c = cast();
    let (a, b) = (made(&c, Caller::WsMember, "a.png", 4), made(&c, Caller::WsMember, "a.png", 4));
    assert_ne!(a.id, b.id);
    assert_ne!(a.url, b.url);
}

#[test]
fn s28_refuses_what_the_pin_refuses_in_the_pins_words() {
    let c = cast();
    let invalid = json!({"error": "Invalid entity type.", "status": false});
    for (what, body, status, want) in [
        ("an entity type that is not one", json!({"type": PNG, "entity_type": "NOPE"}), 400, invalid.clone()),
        ("no entity type", json!({"type": PNG}), 400, invalid.clone()),
        ("an entity type that is not text", json!({"type": PNG, "entity_type": 7}), 400, invalid.clone()),
        (
            "a file type outside the list",
            cover_body("a.svg", "image/svg+xml", 4),
            400,
            json!({"error": "Invalid file type. Only JPEG, PNG, WebP, JPG and GIF files are allowed.", "status": false}),
        ),
        (
            "a type that is null",
            json!({"type": null, "entity_type": "PROJECT_COVER", "entity_identifier": ""}),
            400,
            json!({"error": "Invalid file type. Only JPEG, PNG, WebP, JPG and GIF files are allowed.", "status": false}),
        ),
    ] {
        let r = s28(&c, Caller::WsMember, body);
        assert_eq!((r["status"].as_u64(), &r["body"]), (Some(status), &want), "{what}");
    }
}

#[test]
fn a_workspace_logo_is_for_admins_and_is_not_built_for_anyone_yet() {
    let c = cast();
    let logo = json!({"type": PNG, "size": 4, "entity_type": "WORKSPACE_LOGO", "entity_identifier": ""});
    for who in [Caller::WsMember, Caller::WsGuest, Caller::ProjectAdmin, Caller::ProjectMember, Caller::ProjectGuest] {
        let r = s28(&c, who, logo.clone());
        assert_eq!(r["status"], 403, "{who:?}");
        assert_eq!(r["body"], json!({"error": "Only workspace admins can upload a workspace logo."}), "{who:?}");
    }
    // an admin passes the pin's check; what the pin goes on to do is not built (D-035), and the answer says so
    let r = s28(&c, Caller::WsAdmin, logo);
    assert_eq!(r["status"], 400);
    assert_eq!(r["body"], json!({"error": "This kind of file cannot be uploaded yet.", "status": false}));
}

#[test]
fn the_other_kinds_the_pin_makes_are_refused_not_made() {
    let c = cast();
    for kind in [
        "ISSUE_ATTACHMENT",
        "ISSUE_DESCRIPTION",
        "COMMENT_DESCRIPTION",
        "PAGE_DESCRIPTION",
        "USER_COVER",
        "USER_AVATAR",
        "DRAFT_ISSUE_ATTACHMENT",
        "DRAFT_ISSUE_DESCRIPTION",
    ] {
        let r =
            s28(&c, Caller::WsMember, json!({"type": PNG, "size": 4, "entity_type": kind, "entity_identifier": ""}));
        assert_eq!(
            (r["status"].as_u64(), &r["body"]["error"]),
            (Some(400), &json!("This kind of file cannot be uploaded yet.")),
            "{kind}"
        );
    }
}

#[test]
fn changing_an_existing_projects_cover_through_s28_is_refused_where_the_pin_would_set_it() {
    let c = cast();
    let mut body = cover_body("cover.png", PNG, 70);
    body["entity_identifier"] = json!(Uuid::from_u128(PROJECT).to_string());
    let r = s28(&c, Caller::ProjectAdmin, body);
    assert_eq!(r["status"], 400);
    assert_eq!(r["body"]["error"], "The cover of an existing project cannot be changed this way yet.");
    // what the stock app sends when a project is being made: an empty one; null is none as well
    for none in [json!(""), json!(null)] {
        let mut body = cover_body("cover.png", PNG, 70);
        body["entity_identifier"] = none.clone();
        assert_eq!(s28(&c, Caller::ProjectAdmin, body)["status"], 200, "{none}");
    }
    // and the pin's refusals of what is not an identifier, which are its own (fixtures S28): none given, `false`, or a text that is no id
    let mut without = cover_body("cover.png", PNG, 70);
    without.as_object_mut().unwrap().remove("entity_identifier");
    for (what, body, want) in [
        ("none given", without, json!({"error": "The payload is not valid"})),
        (
            "false",
            {
                let mut b = cover_body("cover.png", PNG, 70);
                b["entity_identifier"] = json!(false);
                b
            },
            json!({"error": "The payload is not valid"}),
        ),
        (
            "a text that is no id",
            {
                let mut b = cover_body("cover.png", PNG, 70);
                b["entity_identifier"] = json!("nope");
                b
            },
            json!({"error": "Please provide valid detail"}),
        ),
        (
            "a number",
            {
                let mut b = cover_body("cover.png", PNG, 70);
                b["entity_identifier"] = json!(7);
                b
            },
            json!({"error": "Please provide valid detail"}),
        ),
    ] {
        let r = s28(&c, Caller::ProjectAdmin, body);
        assert_eq!((r["status"].as_u64(), &r["body"]), (Some(400), &want), "{what}");
    }
}

#[test]
fn the_size_is_clamped_in_silence_defaults_to_the_limit_and_must_be_a_positive_number() {
    let c = cast();
    let length = |body: Value| -> Value {
        let r = s28(&c, Caller::WsMember, body);
        if r["status"] == 200 {
            r["body"]["upload_data"]["headers"]["Content-Length"].clone()
        } else {
            r["body"].clone()
        }
    };
    let typed =
        |size: Value| json!({"type": PNG, "entity_type": "PROJECT_COVER", "entity_identifier": "", "size": size});
    assert_eq!(length(typed(json!(100))), json!("100"));
    assert_eq!(length(typed(json!(LIMIT + 1))), json!(LIMIT.to_string()), "clamped, and the client is not told");
    assert_eq!(length(typed(json!(9_000_000_000_u64))), json!(LIMIT.to_string()));
    assert_eq!(length(typed(json!("1234"))), json!("1234"), "the pin's int() reads a string of digits");
    assert_eq!(length(typed(json!(12.9))), json!("12"), "and cuts a float");
    assert_eq!(
        length(json!({"type": PNG, "entity_type": "PROJECT_COVER", "entity_identifier": ""})),
        json!(LIMIT.to_string()),
        "none is the limit"
    );
    let bad = json!({"error": "Invalid file size.", "status": false});
    for size in [json!(0), json!(-1), json!("big"), json!(null), json!([4]), json!(true)] {
        assert_eq!(length(typed(size.clone())), bad, "{size}");
    }
    // no type is the pin's default
    let r = s28(&c, Caller::WsMember, json!({"entity_type": "PROJECT_COVER", "entity_identifier": "", "size": 4}));
    assert_eq!(r["body"]["upload_data"]["headers"]["Content-Type"], "image/jpeg");
}

// --- the upload --------------------------------------------------------------------------------------------------------------

fn no_grant() -> Value {
    json!({"error": "The upload address is not valid, or it has expired or been used."})
}

#[test]
fn an_upload_is_made_once_to_the_address_it_was_given_with_the_type_and_the_size_it_was_made_for() {
    let c = cast();
    let m = made(&c, Caller::WsMember, "cover.png", 4);
    // wrong in each way, none of them needs a session, and each spends the address
    for (what, content_type, bytes) in [
        ("a type it was not made for", "image/gif", vec![1; 4]),
        ("too many bytes", PNG, vec![1; 5]),
        ("too few", PNG, vec![1; 3]),
    ] {
        let again = made(&c, Caller::WsMember, "cover.png", 4);
        let r = put(&c, &again.url, content_type, bytes);
        assert_eq!(r.status, 403, "{what}");
        assert_eq!(r.json(), json!({"error": "The upload is not the file the address was made for."}), "{what}");
        assert_eq!(put(&c, &again.url, PNG, vec![1; 4]).status, 403, "{what}: a refused upload spends the address");
    }
    // no type at all
    let none = made(&c, Caller::WsMember, "cover.png", 4);
    assert_eq!(raw(&c.router, &c.jar, "", "PUT", &none.url, &[], vec![1; 4]).status, 403);
    // the right one, with a parameter on the type that is not part of it
    let ok = put(&c, &m.url, "image/png; charset=binary", vec![1; 4]);
    assert_eq!((ok.status, ok.body.len()), (200, 0));
    let replay = put(&c, &m.url, PNG, vec![1; 4]);
    assert_eq!((replay.status, replay.json()), (403, no_grant()), "one use");
}

#[test]
fn an_upload_address_names_its_asset_and_only_its_asset() {
    let c = cast();
    let (a, b) = (made(&c, Caller::WsMember, "a.png", 4), made(&c, Caller::WsMember, "b.png", 4));
    let token_of = |m: &Made| m.url.rsplit('/').nth(1).unwrap().to_string();
    let crossed = format!("/api/assets/v2/upload/{}/{}/", a.id, token_of(&b));
    assert_eq!(put(&c, &crossed, PNG, vec![0; 4]).json(), no_grant());
    let unknown = format!("/api/assets/v2/upload/{}/{}/", Uuid::from_u128(0x404), token_of(&a));
    assert_eq!(
        (put(&c, &unknown, PNG, vec![0; 4]).status, put(&c, &unknown, PNG, vec![0; 4]).json()),
        (403, no_grant())
    );
    let wrong_token = format!("/api/assets/v2/upload/{}/{}/", a.id, "0".repeat(64));
    assert_eq!(put(&c, &wrong_token, PNG, vec![0; 4]).status, 403);
    // an id that is no id is a path no route matches
    let r = put(&c, &format!("/api/assets/v2/upload/not-an-id/{}/", token_of(&a)), PNG, vec![0; 4]);
    assert_eq!((r.status, r.json()), (404, json!({"error": "Page not found."})));
    // and none of that touched the real one
    assert_eq!(put(&c, &a.url, PNG, vec![0; 4]).status, 200);
}

#[test]
fn a_body_over_the_limit_is_refused_before_the_address_is_spent() {
    let c = cast();
    let m = made(&c, Caller::WsMember, "big.png", LIMIT);
    assert_eq!(put(&c, &m.url, PNG, vec![0; LIMIT + 1]).status, 413);
    assert_eq!(put(&c, &m.url, PNG, vec![0; LIMIT]).status, 200, "the address still works");
}

// --- S29 ---------------------------------------------------------------------------------------------------------------------

#[test]
fn s29_confirms_an_upload_for_every_member_and_looks_at_the_object_first() {
    let c = cast();
    for who in CALLERS {
        let m = if who.in_workspace() { made(&c, who, "a.png", 4) } else { made(&c, Caller::WsAdmin, "a.png", 4) };
        let path = assets_path(&format!("{}/", m.id));
        // nothing is there yet
        let r = c.call(who, "PATCH", &path, Value::Null);
        match who {
            Caller::Anonymous => {
                assert_eq!((r["status"].as_u64(), &r["body"]), (Some(401), &unauthenticated()), "{who:?}")
            }
            Caller::Stranger => assert_eq!((r["status"].as_u64(), &r["body"]), (Some(403), &by_decorator()), "{who:?}"),
            _ => {
                assert_eq!(
                    (r["status"].as_u64(), &r["body"]),
                    (Some(400), &json!({"error": "The file has not been uploaded."})),
                    "{who:?}"
                );
                assert_eq!(put(&c, &m.url, PNG, vec![9; 4]).status, 200);
                let r = c.call(who, "PATCH", &path, Value::Null);
                assert_eq!((r["status"].as_u64(), &r["body"]), (Some(204), &Value::Null), "{who:?}");
                // confirming again is as good as the first time, as on the pin
                assert_eq!(c.call(who, "PATCH", &path, Value::Null)["status"], 204, "{who:?}");
            }
        }
    }
}

#[test]
fn s29_names_an_asset_that_is_there() {
    let c = cast();
    let r = c.call(Caller::WsMember, "PATCH", &assets_path(&format!("{}/", Uuid::from_u128(0x404))), Value::Null);
    assert_eq!(
        (r["status"].as_u64(), &r["body"]),
        (Some(404), &json!({"error": "The required object does not exist."}))
    );
    let r = c.call(Caller::WsMember, "PATCH", &assets_path("not-an-id/"), Value::Null);
    assert_eq!((r["status"].as_u64(), &r["body"]), (Some(404), &json!({"error": "Page not found."})));
}

// --- S31 ---------------------------------------------------------------------------------------------------------------------

#[test]
fn s31_attaches_a_cover_for_the_projects_people_and_the_app_reads_it_on_the_project() {
    let c = cast();
    for who in CALLERS {
        let r = bulk(&c, who, json!([Uuid::from_u128(0x404).to_string()]));
        let (status, body) = (r["status"].as_u64().unwrap(), &r["body"]);
        match who {
            Caller::Anonymous => assert_eq!((status, body), (401, &unauthenticated()), "{who:?}"),
            // the decorator asks for a place in the project, and a workspace admin who is in none has none
            Caller::Stranger | Caller::WsAdmin | Caller::WsMember | Caller::WsGuest => {
                assert_eq!((status, body), (403, &by_decorator()), "{who:?}")
            }
            _ => assert_eq!(
                (status, body),
                (404, &json!({"error": "The requested asset could not be found."})),
                "{who:?}"
            ),
        }
    }
    for who in [Caller::ProjectAdmin, Caller::ProjectMember, Caller::ProjectGuest] {
        let id = uploaded(&c, who, "cover.png", vec![5; 6]);
        let r = bulk(&c, who, json!([id]));
        assert_eq!((r["status"].as_u64(), &r["body"]), (Some(204), &Value::Null), "{who:?}");
        // what the app opens the project with
        let p = c.call(Caller::ProjectAdmin, "GET", &project_path(""), Value::Null);
        assert_eq!(p["body"]["cover_image_asset"], json!(id), "{who:?}");
        assert_eq!(p["body"]["cover_image_url"], json!(format!("/api/assets/v2/static/{id}/")), "{who:?}");
    }
}

#[test]
fn s31_refuses_an_empty_list_and_leaves_out_what_is_not_the_callers() {
    let c = cast();
    let nothing = json!({"error": "No asset ids provided."});
    // what Python calls false is none; anything else that is no list of ids fails the pin's validation, after its decorator
    for ids in [json!([]), json!(null), json!(""), json!({}), json!(0), json!(false)] {
        let r = bulk(&c, Caller::ProjectMember, ids.clone());
        assert_eq!((r["status"].as_u64(), &r["body"]), (Some(400), &nothing), "{ids}");
    }
    let invalid = json!({"error": "Please provide valid detail"});
    for ids in [
        json!("an-id"),
        json!(["not-an-id"]),
        json!([7]),
        json!(5),
        json!({"a": 1}),
        json!([Uuid::from_u128(9).to_string(), "x"]),
    ] {
        let r = bulk(&c, Caller::ProjectMember, ids.clone());
        assert_eq!((r["status"].as_u64(), &r["body"]), (Some(400), &invalid), "{ids}");
        // and the decorator still comes first: a stranger and a member of the workspace who is not in the project are refused
        for who in [Caller::Stranger, Caller::WsMember] {
            assert_eq!(
                (bulk(&c, who, ids.clone())["status"].as_u64(), bulk(&c, who, ids.clone())["body"].clone()),
                (Some(403), by_decorator()),
                "{who:?} {ids}"
            );
        }
    }
    let r = c.call(
        Caller::ProjectMember,
        "POST",
        &format!("{}{}/{}/bulk/", assets_path("projects/"), Uuid::from_u128(PROJECT), Uuid::from_u128(PROJECT)),
        json!({}),
    );
    assert_eq!((r["status"].as_u64(), &r["body"]), (Some(400), &nothing), "no list at all");

    let theirs = uploaded(&c, Caller::ProjectGuest, "theirs.png", vec![1; 3]);
    let missing = json!({"error": "The requested asset could not be found."});
    for ids in [json!([theirs]), json!([Uuid::from_u128(0x404).to_string()])] {
        let r = bulk(&c, Caller::ProjectMember, ids.clone());
        assert_eq!((r["status"].as_u64(), &r["body"]), (Some(404), &missing), "{ids}");
    }
    assert_eq!(c.project_row().cover_image_asset, None, "none of that wrote anything");
    // named with another's, the caller's own goes through and the other's does not
    let mine = uploaded(&c, Caller::ProjectMember, "mine.png", vec![1; 3]);
    assert_eq!(bulk(&c, Caller::ProjectMember, json!([theirs, mine]))["status"], 204);
    assert_eq!(c.project_row().cover_image_asset.map(|u| u.to_string()), Some(mine));
    // and the other member can still attach theirs: it belongs to no project
    assert_eq!(bulk(&c, Caller::ProjectGuest, json!([theirs]))["status"], 204);
}

// --- S33, the route it sends to, and the address a cover is shown at -----------------------------------------------------------

#[test]
fn s33_sends_a_member_to_the_bytes_and_the_cover_is_shown_to_anyone() {
    let c = cast();
    let bytes: Vec<u8> = (0..70u8).collect();
    let id = uploaded(&c, Caller::WsMember, "../../sneaky/my cover.png", bytes.clone());
    let object = assets_path(&format!("{id}/object/"));
    for who in CALLERS {
        let r = get(&c, who, &assets_path(&format!("{id}/")));
        match who {
            Caller::Anonymous => assert_eq!((r.status, r.json()), (401, unauthenticated()), "{who:?}"),
            Caller::Stranger => assert_eq!((r.status, r.json()), (403, by_decorator()), "{who:?}"),
            _ => {
                assert_eq!((r.status, r.header("location")), (302, object.as_str()), "{who:?}");
                let file = get(&c, who, r.header("location"));
                assert_eq!((file.status, &file.body), (200, &bytes), "{who:?}");
                assert_eq!(file.header("content-type"), PNG);
                // the name the sanitiser kept, as Python's `quote` writes it, and a download, not a page
                assert_eq!(
                    file.header("content-disposition"),
                    "attachment; filename*=UTF-8''my%20cover.png",
                    "{who:?}"
                );
                assert_eq!(file.header("x-content-type-options"), "nosniff");
            }
        }
    }
    // the cover's own address needs no session and shows the picture
    for who in [Caller::Anonymous, Caller::Stranger, Caller::WsMember] {
        let r = get(&c, who, &format!("/api/assets/v2/static/{id}/"));
        assert_eq!((r.status, &r.body), (200, &bytes), "{who:?}");
        assert_eq!(r.header("content-type"), PNG);
        assert_eq!(r.header("content-disposition"), "inline; filename*=UTF-8''my%20cover.png");
        assert_eq!(r.header("x-content-type-options"), "nosniff");
    }
}

#[test]
fn an_asset_is_not_read_before_it_is_uploaded_and_an_unknown_one_is_not_there() {
    let c = cast();
    let pending = made(&c, Caller::WsMember, "a.png", 4);
    put(&c, &pending.url, PNG, vec![1; 4]);
    let not_found = json!({"error": "The requested asset could not be found."});
    // stored but not confirmed: nothing may read it yet
    for path in [assets_path(&format!("{}/", pending.id)), assets_path(&format!("{}/object/", pending.id))] {
        let r = get(&c, Caller::WsMember, &path);
        assert_eq!((r.status, r.json()), (404, not_found.clone()), "{path}");
    }
    let r = get(&c, Caller::Anonymous, &format!("/api/assets/v2/static/{}/", pending.id));
    assert_eq!((r.status, r.json()), (404, not_found.clone()), "the public address says nothing more of a pending one");
    // and an asset that is not there at all
    let ghost = Uuid::from_u128(0x404);
    let r = get(&c, Caller::WsMember, &assets_path(&format!("{ghost}/")));
    assert_eq!((r.status, r.json()), (404, json!({"error": "The required object does not exist."})));
    let r = get(&c, Caller::Anonymous, &format!("/api/assets/v2/static/{ghost}/"));
    assert_eq!((r.status, r.json()), (404, not_found));
    // an id that is no id
    for path in [assets_path("nope/"), assets_path("nope/object/"), "/api/assets/v2/static/nope/".to_string()] {
        let r = get(&c, Caller::WsMember, &path);
        assert_eq!((r.status, r.json()), (404, json!({"error": "Page not found."})), "{path}");
    }
}

#[test]
fn a_project_asset_is_read_and_confirmed_only_by_the_projects_people() {
    let c = cast();
    let id = uploaded(&c, Caller::ProjectMember, "cover.png", vec![3; 5]);
    assert_eq!(bulk(&c, Caller::ProjectMember, json!([id]))["status"], 204);
    let refused = json!({"error": "You don't have access to this asset."});
    for who in CALLERS.into_iter().filter(|w| w.in_workspace()) {
        let (read, confirm) = (
            get(&c, who, &assets_path(&format!("{id}/"))),
            c.call(who, "PATCH", &assets_path(&format!("{id}/")), Value::Null),
        );
        if who.in_project() {
            assert_eq!((read.status, confirm["status"].as_u64()), (302, Some(204)), "{who:?}");
        } else {
            assert_eq!((read.status, read.json()), (403, refused.clone()), "{who:?}");
            assert_eq!((confirm["status"].as_u64(), &confirm["body"]), (Some(403), &refused), "{who:?}");
            // the route it would send them to asks again
            assert_eq!(get(&c, who, &assets_path(&format!("{id}/object/"))).status, 403, "{who:?}");
        }
    }
    // the cover's own address is the pin's AllowAny: it shows the picture to the ones who are in no project too
    assert_eq!(get(&c, Caller::Anonymous, &format!("/api/assets/v2/static/{id}/")).status, 200);
}

// --- a project made or changed with a cover ---------------------------------------------------------------------------------

fn create_project(c: &Cast, who: Caller, name: &str, identifier: &str, extra: Value) -> Value {
    let mut body = json!({"name": name, "identifier": identifier});
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    c.call(who, "POST", &projects_path(), body)
}

fn not_an_asset(id: &str) -> Value {
    json!({"cover_image_asset": [format!("Invalid pk \"{id}\" - object does not exist.")]})
}

#[test]
fn a_project_is_made_with_a_cover_its_maker_uploaded_and_with_no_other() {
    let c = cast();
    let mine = uploaded(&c, Caller::WsMember, "cover.png", vec![1; 4]);
    let theirs = uploaded(&c, Caller::WsAdmin, "cover.png", vec![1; 4]);
    let pending = made(&c, Caller::WsMember, "cover.png", 4).id;
    let ghost = Uuid::from_u128(0x404).to_string();
    for (id, what) in
        [(&theirs, "another member's"), (&pending, "one with no bytes yet"), (&ghost, "one that is no asset")]
    {
        let r = create_project(&c, Caller::WsMember, "Refused", "REF", json!({"cover_image_asset": id}));
        assert_eq!((r["status"].as_u64(), &r["body"]), (Some(400), &not_an_asset(id)), "{what}");
    }
    let r = create_project(&c, Caller::WsMember, "Covered", "COV", json!({"cover_image_asset": mine}));
    assert_eq!(r["status"], 201, "{r}");
    assert_eq!(r["body"]["cover_image_asset"], json!(mine));
    assert_eq!(r["body"]["cover_image_url"], json!(format!("/api/assets/v2/static/{mine}/")));
    // none, null and empty are no cover
    for none in [json!(null), json!("")] {
        let n = format!("None {none}");
        let r = create_project(
            &c,
            Caller::WsMember,
            &n,
            &n.chars().filter(char::is_ascii_alphabetic).collect::<String>().to_uppercase(),
            json!({"cover_image_asset": none}),
        );
        assert_eq!(
            (r["status"].as_u64(), &r["body"]["cover_image_asset"], &r["body"]["cover_image_url"]),
            (Some(201), &Value::Null, &Value::Null),
            "{none}"
        );
    }
}

#[test]
fn s32_changes_the_cover_clears_it_and_ignores_the_read_only_url() {
    let c = cast();
    let first = uploaded(&c, Caller::ProjectAdmin, "one.png", vec![1; 4]);
    let second = uploaded(&c, Caller::ProjectAdmin, "two.png", vec![2; 4]);
    let patch = |who: Caller, body: Value| c.call(who, "PATCH", &project_path(""), body);

    let r = patch(Caller::ProjectAdmin, json!({"cover_image_asset": first}));
    assert_eq!((r["status"].as_u64(), &r["body"]["cover_image_asset"]), (Some(200), &json!(first)), "{r}");
    // the url is the pin's property: sent, it is ignored
    let r = patch(Caller::ProjectAdmin, json!({"cover_image_url": "https://elsewhere.test/x.png"}));
    assert_eq!(r["body"]["cover_image_url"], json!(format!("/api/assets/v2/static/{first}/")));
    // a save by someone else that repeats the cover the project has is fine, whoever uploaded it
    assert_eq!(bulk(&c, Caller::ProjectAdmin, json!([first]))["status"], 204);
    let r = patch(Caller::WsAdmin, json!({"cover_image_asset": first, "name": "Renamed"}));
    assert_eq!(r["status"], 200, "{r}");
    // but naming another member's upload that no project has is not
    let r = patch(Caller::WsAdmin, json!({"cover_image_asset": second}));
    assert_eq!((r["status"].as_u64(), &r["body"]), (Some(400), &not_an_asset(&second)));
    // the maker may swap it, and clear it
    let r = patch(Caller::ProjectAdmin, json!({"cover_image_asset": second}));
    assert_eq!((r["status"].as_u64(), &r["body"]["cover_image_asset"]), (Some(200), &json!(second)));
    let r = patch(Caller::ProjectAdmin, json!({"cover_image_asset": null}));
    assert_eq!(
        (r["body"]["cover_image_asset"].clone(), r["body"]["cover_image_url"].clone()),
        (Value::Null, Value::Null)
    );
    // an id that is no id is the field's error, not the whole request's
    let r = patch(Caller::ProjectAdmin, json!({"cover_image_asset": "nope"}));
    assert_eq!(
        (r["status"].as_u64(), &r["body"]["cover_image_asset"][0]),
        (Some(400), &json!("\u{201c}nope\u{201d} is not a valid UUID."))
    );
}

#[test]
fn a_cover_of_text_is_the_url_when_there_is_no_asset_and_none_when_it_is_empty() {
    let c = cast();
    let mut row = c.project_row();
    row.cover_image = Some("https://images.example.test/c.jpg".into());
    c.save(&row);
    let p = c.call(Caller::ProjectMember, "GET", &project_path(""), Value::Null);
    assert_eq!(p["body"]["cover_image_url"], "https://images.example.test/c.jpg");
    row.cover_image = Some(String::new());
    c.save(&row);
    let p = c.call(Caller::ProjectMember, "GET", &project_path(""), Value::Null);
    assert_eq!(p["body"]["cover_image_url"], Value::Null, "the pin's property answers None for an empty text");
}

// --- the whole journey the stock app makes ----------------------------------------------------------------------------------

#[test]
fn the_stock_apps_project_creation_runs_end_to_end() {
    let c = cast();
    let me = Caller::WsMember;
    let picture: Vec<u8> = (0..200u8).rev().collect();
    // 1. S28, with the empty entity identifier the app sends for a project that does not exist yet
    let m = made(&c, me, "image_1.png", picture.len());
    // 2. the browser's PUT, no cookies, the headers S28 named less Content-Length
    assert_eq!(put(&c, &m.url, PNG, picture.clone()).status, 200);
    // 3. S29
    assert_eq!(c.call(me, "PATCH", &assets_path(&format!("{}/", m.id)), Value::Null)["status"], 204);
    // 4. S30, without the cover (the app attaches it after)
    let created = create_project(&c, me, "Journey", "JRN", json!({}));
    assert_eq!(created["status"], 201);
    let project = created["body"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["body"]["cover_image_url"], Value::Null);
    // 5. S31
    let path = format!("/api/assets/v2/workspaces/{SLUG}/projects/{project}/{project}/bulk/");
    assert_eq!(c.call(me, "POST", &path, json!({"asset_ids": [m.id]}))["status"], 204);
    // 6. S32 with the url the app repeats, which is read-only
    let url = format!("/api/assets/v2/static/{}/", m.id);
    let patched = c.call(me, "PATCH", &format!("{}{project}/", projects_path()), json!({"cover_image_url": url}));
    assert_eq!(patched["status"], 200, "{patched}");
    // 7. S34, the open project, and the picture its url names
    let opened = c.call(me, "GET", &format!("{}{project}/", projects_path()), Value::Null);
    assert_eq!(opened["body"]["cover_image_url"], json!(url));
    assert_eq!(opened["body"]["cover_image_asset"], json!(m.id));
    let shown = get(&c, Caller::Anonymous, &url);
    assert_eq!((shown.status, shown.body), (200, picture));
    // and the list the sidebar reads names the project (S17), which has no cover of its own to show
    let list = c.call(me, "GET", &projects_path(), Value::Null);
    assert!(list["body"].as_array().unwrap().iter().any(|p| p["id"] == json!(project)));
}
