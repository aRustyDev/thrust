//! One `parity_` test per compat row: the route answers what the pinned Plane (`oracle/v1-pin-20261018`, `ba85d4d3d3`)
//! recorded, after both are normalised by the same rules (tests/parity/normalise.json). The fixtures are output recorded
//! from that independent oracle by tools/compat-record/record.py; they are never edited by hand (AGENTS.md testing rule
//! 9c). A row's fixture holds every exchange the issue's acceptance criteria name for it, the 401/403/404 ones included.
//!
//! The rows are one journey, in the order of `rows!` below: each test replays the earlier rows on the same world first,
//! so a sign-in finds the user that the sign-up made, as it did on the pin. The sign-in and sign-up rows are compared
//! through the projection of the pin's 302 onto Thrust's JSON (D-022), documented in tests/common/mod.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use axum::body::Body;
use axum::http::Response;
use common::*;
use serde_json::{json, Value};

/// `parity_<row>` per row, and `ROWS`, the journey in order, which the guard below holds the fixtures directory to.
macro_rules! rows {
    ($($test:ident => $row:literal),+ $(,)?) => {
        const ROWS: &[&str] = &[$($row),+];
        $(
            #[test]
            fn $test() {
                let d = journey(ROWS, $row, |_| {});
                assert!(d.is_empty(), "row {} diverges from the pinned Plane:\n{}", $row, d.join("\n"));
            }
        )+
    };
}

rows! {
    parity_s01_instances => "S01",
    parity_s02_users_me_signed_out => "S02",
    parity_s03_csrf_token => "S03",
    parity_s04_email_check => "S04",
    parity_s05_sign_up => "S05",
    parity_s04_email_check_after_sign_up => "S04-existing",
    parity_s06_sign_in => "S06",
    parity_s02_users_me_signed_in => "S02-signed-in",
    parity_s07_profile => "S07",
    parity_s10_update_user => "S10",
    parity_s11_update_profile => "S11",
    parity_s27_tour_completed => "S27",
    parity_x04_onboard => "X04",
    parity_s12_invitations => "S12",
    parity_s09_my_workspaces => "S09",
    parity_s08_settings => "S08",
    parity_s13_slug_check => "S13",
    parity_s14_create_workspace => "S14",
    parity_s01_instances_once_a_workspace_exists => "S01-after",
    parity_s13_slug_check_after => "S13-after",
    parity_s09_my_workspaces_after => "S09-after",
    parity_s08_settings_after => "S08-after",
    parity_s15_member_me => "S15",
    parity_s16_project_roles => "S16",
    parity_s17_projects => "S17",
    parity_s18_members => "S18",
    parity_s19_user_favorites => "S19",
    parity_s20_states => "S20",
    parity_s21_sidebar_preferences => "S21",
    parity_s22_user_properties => "S22",
    parity_s23_unread_notifications => "S23",
    parity_s24_home_preferences => "S24",
    parity_s25_quick_links => "S25",
    parity_s26_recent_visits => "S26",
    parity_s30_create_project => "S30",
    parity_s32_update_project => "S32",
    parity_s34_open_project => "S34",
    parity_s35_project_stats => "S35",
    parity_s36_project_member_me => "S36",
    parity_s37_project_user_properties => "S37",
    parity_s38_project_labels => "S38",
    parity_s39_project_members => "S39",
    parity_s40_project_states => "S40",
    parity_s41_intake_state => "S41",
    parity_s42_estimates => "S42",
    parity_s43_cycles => "S43",
    parity_s44_workspace_modules => "S44",
    parity_s45_project_modules => "S45",
    parity_s46_project_views => "S46",
    parity_s17_projects_once_they_exist => "S17-after",
    parity_s20_states_once_they_exist => "S20-after",
    parity_s48_create_work_item => "S48",
    parity_s47_list_work_items => "S47",
    parity_x01_open_work_item => "X01",
    parity_x03_change_a_work_item => "X03",
    parity_x02_work_item_history => "X02",
    parity_s28_upload_an_asset => "S28",
    parity_s29_confirm_an_upload => "S29",
    parity_s31_attach_assets_to_a_project => "S31",
    parity_s34_open_project_once_it_has_a_cover => "S34-after",
    parity_s33_read_an_asset => "S33",
    parity_x05_a_covers_own_address => "X05",
    parity_x07_stickies => "X07",
    parity_x08_unsplash => "X08",
    parity_s28_upload_an_asset_as_a_member => "S28-member",
    parity_s29_confirm_an_upload_as_a_member => "S29-member",
    parity_s31_attach_assets_as_a_member => "S31-member",
    parity_s33_read_an_asset_as_a_member => "S33-member",
    parity_x06_sign_out => "X06",
    parity_x09_change_password => "X09",
    parity_x10_forgot_password_without_smtp => "X10-dark",
    parity_x10_forgot_password => "X10",
    parity_x11_reset_password => "X11",
}

/// A fixture without a test would pass unseen, and a test without a fixture is `load_fixture` panicking: hold the two
/// lists equal, so adding a row means adding both.
#[test]
fn parity_every_fixture_has_a_test() {
    let mut rows: Vec<String> = ROWS.iter().map(|r| r.to_string()).collect();
    rows.sort();
    assert_eq!(fixture_rows(), rows, "tests/parity/fixtures/*.json and the rows! list must name the same rows");
}

/// The acceptance list (issue #51): the 48-route slice (S01 to S48), X01 to X03 and X04. It is `tests/parity/acceptance.csv`, written
/// by tools/compat-record/gen-acceptance.py from research/0003 section 8.1, and `scripts/check-acceptance-list.sh` also holds the
/// recorder to it. Its `state` column is data and not derived from the fixtures, so this compares two independent things.
const ACCEPTANCE: &str = include_str!("parity/acceptance.csv");

/// Where the list and the replay's rows (`S04-existing` is row `S04` again) disagree: a `served` row nothing replays, a replayed row
/// that the list does not call served, and a `pending` row that is replayed (a row that is not served is not recorded).
fn acceptance_divergences(csv: &str, rows: &[&str]) -> Vec<String> {
    use std::collections::BTreeSet;
    let (mut served, mut pending) = (BTreeSet::new(), BTreeSet::new());
    for line in csv.lines().skip(1) {
        let mut fields = line.split(',');
        // row,method,path,state
        let (Some(row), Some(state)) = (fields.next(), fields.nth(2)) else { continue };
        let set = if state == "served" { &mut served } else { &mut pending };
        set.insert(row.to_string());
    }
    let replayed: BTreeSet<String> = rows.iter().map(|r| r.split('-').next().unwrap_or(r).to_string()).collect();
    let mut out = Vec::new();
    if served.len() < 40 {
        out.push(format!(
            "{} served rows: a list that small is not the acceptance list, and the comparison would prove nothing",
            served.len()
        ));
    }
    out.extend(served.difference(&replayed).map(|r| format!("{r} is served and has no parity_ test")));
    out.extend(
        replayed.difference(&served).map(|r| format!("{r} has a parity_ test and the list does not call it served")),
    );
    out.extend(pending.intersection(&replayed).map(|r| format!("{r} is pending and has a parity_ test")));
    out
}

#[test]
fn parity_the_acceptance_list_and_the_replay_name_the_same_rows() {
    let d = acceptance_divergences(ACCEPTANCE, ROWS);
    assert!(d.is_empty(), "tests/parity/acceptance.csv and the rows! list disagree:\n{}", d.join("\n"));
}

/// Controls (AGENTS.md testing rule 8): the comparison can fail, each way it is for.
#[test]
fn parity_control_the_acceptance_list_can_disagree_with_the_replay() {
    let edited = |f: &dyn Fn(&str) -> Option<String>| -> String {
        ACCEPTANCE.lines().filter_map(f).map(|l| format!("{l}\n")).collect()
    };
    assert!(acceptance_divergences(ACCEPTANCE, ROWS).is_empty(), "the baseline");

    let pending =
        edited(&|l| Some(if l.starts_with("S45,") { l.replace(",served", ",pending-x") } else { l.to_string() }));
    let d = acceptance_divergences(&pending, ROWS);
    assert!(
        d.iter().any(|m| m.starts_with("S45 has a parity_ test")) && d.iter().any(|m| m.starts_with("S45 is pending")),
        "{d:?}"
    );

    let without = edited(&|l| (!l.starts_with("S45,")).then(|| l.to_string()));
    let d = acceptance_divergences(&without, ROWS);
    assert!(d.iter().any(|m| m.starts_with("S45 has a parity_ test")), "{d:?}");

    let more = format!("{ACCEPTANCE}S99,GET,/api/nothing/,served\n");
    let d = acceptance_divergences(&more, ROWS);
    assert!(d.iter().any(|m| m.starts_with("S99 is served and has no parity_ test")), "{d:?}");

    let d = acceptance_divergences("row,method,path,state\n", &[]);
    assert!(d.iter().any(|m| m.contains("not the acceptance list")), "an empty list is not a pass: {d:?}");
}

// ---------------------------------------------------------------------------------------------------------------------
// Controls (AGENTS.md testing rule 8): the harness must be able to fail. Each mutates a recording in memory and
// requires the replay to report a divergence; a pass here with the mutation unseen would void every row above.
// ---------------------------------------------------------------------------------------------------------------------

fn mutated(row: &str, edit: impl FnOnce(&mut Value)) -> Vec<String> {
    journey(ROWS, row, edit)
}

#[test]
fn parity_control_unmutated_fixtures_have_no_divergence() {
    // the baselines the controls below are measured against
    for row in ["S01", "S05", "S06"] {
        assert!(mutated(row, |_| {}).is_empty(), "{row}");
    }
}

#[test]
fn parity_control_renamed_field_is_a_divergence() {
    let d = mutated("S01", |f| {
        let instance = &mut f["exchanges"][0]["response"]["body"]["instance"];
        let v = instance.as_object_mut().unwrap().remove("is_setup_done").unwrap();
        instance["is_setup_complete"] = v;
    });
    assert!(!d.is_empty());
}

#[test]
fn parity_control_changed_status_is_a_divergence() {
    assert!(!mutated("S02", |f| f["exchanges"][0]["response"]["status"] = json!(403)).is_empty());
}

#[test]
fn parity_control_changed_value_is_a_divergence() {
    let d = mutated("S01", |f| {
        f["exchanges"][0]["response"]["body"]["config"]["enable_signup"] = json!(false);
    });
    assert!(!d.is_empty());
}

#[test]
fn parity_control_changed_type_is_a_divergence() {
    // Plane sends file_size_limit as a float; 5242880 and 5242880.0 are different JSON numbers to serde_json
    let d = mutated("S01", |f| {
        f["exchanges"][0]["response"]["body"]["config"]["file_size_limit"] = json!(5_242_880);
    });
    assert!(!d.is_empty());
}

/// The projection must not make every 302 equal: a recorded failure that the pin reports as another code is a
/// divergence, and so is one the pin reports as a success.
#[test]
fn parity_control_a_different_error_code_in_a_redirect_is_a_divergence() {
    let d = mutated("S06", |f| {
        let ex = f["exchanges"].as_array_mut().unwrap().iter_mut().find(|e| e["name"] == "wrong password").unwrap();
        let loc = ex["response"]["headers"]["location"].as_str().unwrap().replace("error_code=5065", "error_code=5060");
        ex["response"]["headers"]["location"] = json!(loc);
    });
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0].contains("error_code"), "{d:?}");
}

#[test]
fn parity_control_a_recorded_success_that_thrust_refuses_is_a_divergence() {
    let d = mutated("S06", |f| {
        let ex = f["exchanges"].as_array_mut().unwrap().iter_mut().find(|e| e["name"] == "wrong password").unwrap();
        ex["response"]["headers"]["location"] = json!("<base_url>");
    });
    assert!(!d.is_empty());
}

#[test]
fn parity_control_a_next_path_that_is_not_kept_is_a_divergence() {
    let d = mutated("S06", |f| {
        let ex = f["exchanges"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|e| e["name"] == "sign in with a next_path")
            .unwrap();
        ex["response"]["headers"]["location"] = json!("<base_url>/?next_path=/elsewhere");
    });
    assert!(!d.is_empty());
}

/// The recorded password is what the sign-in tries: a recording whose sign-in uses another password than the sign-up
/// set is refused by Thrust, so the replay does not just assume success.
#[test]
fn parity_control_a_sign_in_with_the_wrong_password_is_a_divergence() {
    let d = mutated("S06", |f| {
        let ex = f["exchanges"].as_array_mut().unwrap().iter_mut().find(|e| e["name"] == "sign in").unwrap();
        ex["request"]["body"]["form"]["password"] = json!("Some-Other-Pa55word-Entirely-7");
    });
    assert!(!d.is_empty());
}

/// A sign-in that works must set the session cookie, and that check must be able to fail: the same replay through a
/// router that strips every `Set-Cookie` header diverges, at the sign-ups and sign-ins the recording says worked.
#[test]
fn parity_control_a_sign_in_without_a_session_cookie_is_a_divergence() {
    use tower::util::MapResponseLayer;
    let strip = MapResponseLayer::new(|mut r: Response<Body>| {
        r.headers_mut().remove("set-cookie");
        r
    });
    let router = app().layer(strip);
    let d = divergences(&router, &load_fixture("S05"));
    assert!(d.iter().any(|m| m.contains("session cookie")), "{d:?}");
}

/// A sign-out clears both cookies (D-036), which the fixture drops with the other headers, so the replay checks it itself: the
/// same replay through a router that strips the `Set-Cookie` headers of the sign-out's answers (and only those: the sign-ins before
/// it need theirs for the journey to hold a session at all) must diverge at each sign-out.
#[test]
fn parity_control_a_sign_out_that_clears_no_cookie_is_a_divergence() {
    use axum::{extract::Request, middleware::Next};
    async fn strip_the_sign_outs_cookies(req: Request, next: Next) -> Response<Body> {
        let signs_out = req.uri().path() == "/auth/sign-out/";
        let mut resp = next.run(req).await;
        if signs_out {
            resp.headers_mut().remove("set-cookie");
        }
        resp
    }
    let d = journey_with(
        |router| router.layer(axum::middleware::from_fn(strip_the_sign_outs_cookies)),
        ROWS,
        "X06",
        |_| {},
    );
    let cookie_lines = d.iter().filter(|m| m.contains("cookies: a sign-out must clear")).count();
    assert_eq!(cookie_lines, 2, "the two sign-outs that were answered, not the read between them: {d:?}");
}

/// The read after the first sign-out sends the session the account held before it, so its 401 is the end of that session and
/// nothing else. A router whose sign-out answers as it should but ends nothing must therefore diverge at that read.
#[test]
fn parity_control_a_sign_out_that_ends_no_session_is_a_divergence() {
    use axum::{extract::Request, http::StatusCode, middleware::Next};
    async fn answer_without_doing(req: Request, next: Next) -> Response<Body> {
        if req.uri().path() != "/auth/sign-out/" {
            return next.run(req).await;
        }
        let mut resp = Response::new(Body::from(r#"{"redirect_path":"/"}"#));
        *resp.status_mut() = StatusCode::OK;
        resp.headers_mut().insert("content-type", "application/json".parse().unwrap());
        for cookie in ["__Host-thrust_session=; Max-Age=0; Path=/", "__Host-thrust_csrf=; Max-Age=0; Path=/"] {
            resp.headers_mut().append("set-cookie", cookie.parse().unwrap());
        }
        resp
    }
    let d = journey_with(|router| router.layer(axum::middleware::from_fn(answer_without_doing)), ROWS, "X06", |_| {});
    assert!(d.iter().any(|m| m.contains("the session is over")), "{d:?}");
    assert!(!d.iter().any(|m| m.contains("a live session")), "the sign-out's own answer is as recorded: {d:?}");
}

/// What the recorder (tools/compat-record/record.py, `step_x09`) sent to change a password: the old one the new account was made with,
/// the new one, and a wrong one. A control that needs to find the successful change in a request, or the wrong guess, looks for them.
const X09_OLD: &str = "Journey-Sample-Pa55word-Zebra-81";
const X09_NEW: &str = "Another-Sample-Pa55word-Yak-27";
const X09_WRONG: &str = "Wrong-Password-For-Journey-1";

/// A change of password sets the caller's new session (D-039), which the fixture drops with the other headers, so the replay checks
/// it itself: through a router that strips the `Set-Cookie` headers of the change's answers, each of the two that worked (the change,
/// and the same password again) must diverge. A refusal has nothing to strip, so it does not.
#[test]
fn parity_control_a_change_that_sets_no_session_cookie_is_a_divergence() {
    use axum::{extract::Request, middleware::Next};
    async fn strip_the_changes_cookies(req: Request, next: Next) -> Response<Body> {
        let changes = req.uri().path() == "/auth/change-password/";
        let mut resp = next.run(req).await;
        if changes {
            resp.headers_mut().remove("set-cookie");
        }
        resp
    }
    let d =
        journey_with(|router| router.layer(axum::middleware::from_fn(strip_the_changes_cookies)), ROWS, "X09", |_| {});
    let cookie_lines = d.iter().filter(|m| m.contains("session cookie: the recording says it must be set")).count();
    assert_eq!(cookie_lines, 2, "the two answers that changed the password: {d:?}");
}

/// The read after the change sends the session the account held before it, so its 401 is the end of that session and nothing else.
/// A router whose change answers as it should, session cookie and all, but ends nothing must diverge at that read (and only the
/// change that would have been made is answered so: the refusals before it are the handler's own).
#[test]
fn parity_control_a_change_that_ends_no_session_is_a_divergence() {
    use axum::{extract::Request, http::StatusCode, middleware::Next};
    async fn answer_without_doing(req: Request, next: Next) -> Response<Body> {
        if req.uri().path() != "/auth/change-password/" {
            return next.run(req).await;
        }
        let (parts, body) = req.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&bytes).to_string();
        let is_the_change = text.contains(&format!("\"old_password\":\"{X09_OLD}\""))
            && text.contains(&format!("\"new_password\":\"{X09_NEW}\""));
        if !is_the_change {
            return next.run(Request::from_parts(parts, Body::from(bytes))).await;
        }
        let mut resp = Response::new(Body::from(r#"{"message":"Password updated successfully"}"#));
        *resp.status_mut() = StatusCode::OK;
        resp.headers_mut().insert("content-type", "application/json".parse().unwrap());
        for cookie in ["__Host-thrust_session=made-up; Path=/", "__Host-thrust_csrf=made-up; Path=/"] {
            resp.headers_mut().append("set-cookie", cookie.parse().unwrap());
        }
        resp
    }
    let d = journey_with(|router| router.layer(axum::middleware::from_fn(answer_without_doing)), ROWS, "X09", |_| {});
    assert!(d.iter().any(|m| m.contains("the session held before the change")), "{d:?}");
    assert!(!d.iter().any(|m| m.contains("exchange \"the change\":")), "the change's own answer is as recorded: {d:?}");
}

/// A wrong old password is refused (5135). A router that mistakes every wrong guess for the right one, which is what a handler that
/// does not check the old password does, must diverge at the exchanges that send one: the recording says 400, and this one changes
/// the password.
#[test]
fn parity_control_a_wrong_old_password_that_is_accepted_is_a_divergence() {
    use axum::{extract::Request, middleware::Next};
    async fn mistake_the_guess_for_the_password(req: Request, next: Next) -> Response<Body> {
        if req.uri().path() != "/auth/change-password/" {
            return next.run(req).await;
        }
        let (parts, body) = req.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&bytes).replace(X09_WRONG, X09_OLD);
        next.run(Request::from_parts(parts, Body::from(text))).await
    }
    let d = journey_with(
        |router| router.layer(axum::middleware::from_fn(mistake_the_guess_for_the_password)),
        ROWS,
        "X09",
        |_| {},
    );
    assert!(d.iter().any(|m| m.contains("exchange \"a wrong old password\":")), "{d:?}");
    // control: the layer touches no other refusal's request, so the ones that name no wrong guess are as recorded
    assert!(!d.iter().any(|m| m.contains("exchange \"no old password\":")), "{d:?}");
}

/// The normaliser could erase a real difference (the failure mode its rules exist to avoid): two answers that differ
/// in an id's *shape of use* must still differ. Here the same id appearing twice versus two different ids.
#[test]
fn parity_control_normaliser_keeps_id_identity() {
    let rules = normalise::Rules::load();
    let ex = |a: &str, b: &str| {
        json!({"name": "n", "request": {"method": "GET", "path": "/"},
               "response": {"status": 200, "headers": {}, "body": {"x": a, "y": b}}})
    };
    let (u1, u2) = ("11111111-1111-1111-1111-111111111111", "22222222-2222-2222-2222-222222222222");
    let same = normalise::exchange(&rules, &mut normalise::Ids::new(&rules), &ex(u1, u1));
    let different = normalise::exchange(&rules, &mut normalise::Ids::new(&rules), &ex(u1, u2));
    assert_ne!(same, different);
}

/// What the projection does to the pin's redirects, pinned on the shapes the pin was seen to produce (spec examples; the
/// fixtures above exercise it on every recorded exchange).
#[test]
fn parity_projection_of_the_pins_redirects() {
    let redirect = |loc: &str| {
        project(&json!({
            "name": "x",
            "request": {"method": "POST", "path": "/auth/sign-in/", "body": {"form": {"email": "a@example.test"}}},
            "response": {"status": 302, "headers": {"location": loc}, "body": null}
        }))
    };
    let r = redirect("<base_url>");
    assert_eq!(r["request"]["body"], json!({"email": "a@example.test"}), "the form becomes JSON");
    assert_eq!((&r["response"]["status"], &r["response"]["body"]), (&json!(200), &json!({"redirect_path": "/"})));

    let r = redirect("<base_url>/?next_path=/dashboard");
    assert_eq!(r["response"]["body"], json!({"redirect_path": "/?next_path=/dashboard"}));

    let r = redirect("<base_url>/?error_code=5021&error_message=PASSWORD_TOO_WEAK&email=a%40example.test");
    assert_eq!(r["response"]["status"], 400);
    assert_eq!(
        r["response"]["body"],
        json!({"error_code": 5021, "error_message": "PASSWORD_TOO_WEAK", "email": "a@example.test"})
    );

    // an error on a request that had a next_path: the path is the client's own and is not part of the error
    let r =
        redirect("<base_url>/?next_path=/dashboard&error_code=5065&error_message=AUTHENTICATION_FAILED_SIGN_IN&email=");
    assert_eq!(
        r["response"]["body"],
        json!({"error_code": 5065, "error_message": "AUTHENTICATION_FAILED_SIGN_IN", "email": ""})
    );

    // the pin's throttle is a 429
    assert_eq!(redirect("<base_url>/?error_code=5900&error_message=RATE_LIMIT_EXCEEDED")["response"]["status"], 429);
}

/// The cases the Python normaliser (which records the fixtures) passes in `record.py --self-test`.
#[test]
fn parity_normaliser_cases() {
    let cases: Value = serde_json::from_str(include_str!("parity/normalise-cases.json")).unwrap();
    let rules = normalise::Rules::load();
    let cases = cases["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let mut ids = normalise::Ids::new(&rules);
        let got: Vec<Value> =
            case["input"].as_array().unwrap().iter().map(|ex| normalise::exchange(&rules, &mut ids, ex)).collect();
        assert_eq!(Value::Array(got), case["expected"], "case: {}", case["name"]);
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The asset stage's controls (AGENTS.md rule 8). Its rows are compared through a projection (D-035, tests/common/mod.rs) that
// drops what is the object store's and renumbers the ids it left out: each control makes one thing wrong in the recording and
// requires the replay to see it, so that the projection is shown not to hide what it does not name.
// ---------------------------------------------------------------------------------------------------------------------

fn exchange_named<'a>(fixture: &'a mut Value, name: &str) -> &'a mut Value {
    fixture["exchanges"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["name"] == name)
        .unwrap_or_else(|| panic!("{name}"))
}

/// What the app reads of the upload (the method and the headers it must send) is compared, though the address is not.
#[test]
fn parity_control_s28_a_changed_upload_method_or_header_is_a_divergence() {
    for (what, edit) in [
        ("method", (|u: &mut Value| u["method"] = json!("POST")) as fn(&mut Value)),
        ("content type", |u| u["headers"]["Content-Type"] = json!("image/gif")),
        ("content length", |u| u["headers"]["Content-Length"] = json!("71")),
        ("fields", |u| u["fields"] = json!({"key": "x"})),
    ] {
        let d = mutated("S28", |f| {
            edit(&mut exchange_named(f, "the project cover, as the app sends it")["response"]["body"]["upload_data"])
        });
        assert_eq!(d.len(), 1, "{what}: {d:?}");
    }
}

/// A recorded refusal that Thrust answers as a success, and the other way, is a divergence: the nine kinds it does not make are
/// compared as the 400 it gives, and a cover is still compared as what the pin made.
#[test]
fn parity_control_s28_a_refusal_and_a_success_are_not_swapped() {
    let d = mutated("S28", |f| exchange_named(f, "an entity type: USER_AVATAR")["response"]["status"] = json!(200));
    // the projection turns that 200 into the 400, and Thrust gives it: no difference, which is the rule working
    assert!(d.is_empty(), "{d:?}");
    let d = mutated("S28", |f| {
        let ex = exchange_named(f, "the project cover, as the app sends it");
        ex["request"]["body"]["entity_type"] = json!("USER_AVATAR");
    });
    assert!(d.is_empty(), "{d:?}");
    // a refusal of the pin that Thrust does not give is seen
    let d = mutated("S28", |f| {
        let ex = f["exchanges"].as_array_mut().unwrap().iter_mut().find(|e| e["response"]["status"] == 400).unwrap();
        ex["response"]["status"] = json!(200);
    });
    assert!(!d.is_empty());
}

/// The address a cover is shown at is compared as the type the pin pins on the picture, which Thrust must serve with.
#[test]
fn parity_control_x05_the_type_pinned_on_a_picture_is_compared() {
    let d = mutated("X05", |f| {
        let ex = exchange_named(f, "no session: the route is public");
        let loc = ex["response"]["headers"]["location"]
            .as_str()
            .unwrap()
            .replace("response-content-type=image%2Fpng", "response-content-type=image%2Fgif");
        ex["response"]["headers"]["location"] = json!(loc);
    });
    assert_eq!(d.len(), 1, "{d:?}");
}

/// S33's answers are compared past its redirect: a recorded success that is a refusal here, or a refusal that is a success, differs.
#[test]
fn parity_control_s33_a_member_outside_the_project_is_still_refused() {
    let d = mutated("S33-member", |f| {
        let ex = f["exchanges"].as_array_mut().unwrap().iter_mut().find(|e| e["response"]["status"] == 403).unwrap();
        ex["response"]["status"] = json!(302);
        ex["response"]["headers"]["location"] =
            json!("<store_url>/uploads/<uuid:5>/<uuid:6>-cover.png?X-Amz-Signature=<signature>");
    });
    assert!(!d.is_empty());
}

/// An exception for the pin's not looking at the object must be one: the pin answers something other than what Thrust does there.
/// An entry that names no exchange, or one the pin answers as Thrust does, would hide nothing and say a deviation that is not one.
#[test]
fn parity_every_looks_at_the_object_entry_names_an_exchange_the_pin_answers_differently() {
    for (row, name, status, body) in LOOKS_AT_THE_OBJECT {
        let mut fixture = load_fixture(row);
        let recorded = exchange_named(&mut fixture, name)["response"].clone();
        let thrust: Value = serde_json::from_str(body).unwrap();
        assert_ne!(
            (recorded["status"].as_u64(), &recorded["body"]),
            (Some(u64::from(*status)), &thrust),
            "{row}, {name}: the pin answers as Thrust does"
        );
    }
}

/// The ids a projection drops are taken out of the numbering, so the ones after them are numbered as Thrust, which makes none of
/// them, numbers them: by their rank among those that remain, the first `kept` always among them.
#[test]
fn parity_control_the_numbering_closes_up_over_what_the_projection_drops() {
    let ex = |path: &str, body: &str| json!({"request": {"path": path}, "response": {"body": body}});
    let out = compact_ids(vec![ex("/x/<uuid:1>/", "<uuid:3> and <uuid:5>"), ex("/y/<uuid:5>", "<uuid:7> <uuid:3>")], 1);
    assert_eq!(out[0], ex("/x/<uuid:1>/", "<uuid:2> and <uuid:3>"));
    assert_eq!(out[1], ex("/y/<uuid:3>", "<uuid:4> <uuid:2>"));
    // an id the fixture uses stays where it is even when nothing in it names it
    let out = compact_ids(vec![ex("/z/", "<uuid:4>")], 3);
    assert_eq!(out[0], ex("/z/", "<uuid:4>"));
}

// --- controls for X10 and X11 (D-041): what the replay must be able to see ---------------------------------------------------------

/// A reset signs nobody in, on the pin and here. A router that sets a session cookie on every answer of the reset form must diverge
/// at each of them (the cookie is transport, which the normaliser drops, so the replay checks it itself).
#[test]
fn parity_control_a_reset_that_signs_somebody_in_is_a_divergence() {
    use axum::{extract::Request, middleware::Next};
    async fn sign_them_in(req: Request, next: Next) -> Response<Body> {
        let reset = req.uri().path().starts_with("/auth/reset-password/");
        let mut resp = next.run(req).await;
        if reset {
            resp.headers_mut().append("set-cookie", "__Host-thrust_session=made-up; Path=/".parse().unwrap());
        }
        resp
    }
    let d = journey_with(|router| router.layer(axum::middleware::from_fn(sign_them_in)), ROWS, "X11", |_| {});
    let lines = d.iter().filter(|m| m.contains("a reset signs nobody in, and this answer set one")).count();
    assert_eq!(lines, 10, "the eight refusals, the reset and the link again: {d:?}");
}

/// A refusal's redirect carries the code the page reads. A router that drops `error_code` from every redirect of the reset form must
/// diverge at every refusal, and not at the success (which has none to drop).
#[test]
fn parity_control_a_refused_reset_without_its_code_is_a_divergence() {
    use axum::{extract::Request, middleware::Next};
    async fn drop_the_code(req: Request, next: Next) -> Response<Body> {
        let reset = req.uri().path().starts_with("/auth/reset-password/");
        let mut resp = next.run(req).await;
        if reset {
            if let Some(location) =
                resp.headers().get("location").and_then(|v| v.to_str().ok()).map(|l| l.replace("error_code=", "code="))
            {
                resp.headers_mut().insert("location", location.parse().unwrap());
            }
        }
        resp
    }
    let d = journey_with(|router| router.layer(axum::middleware::from_fn(drop_the_code)), ROWS, "X11", |_| {});
    let diverged = d.iter().filter(|m| m.starts_with("exchange ")).count();
    // the eight refusals and the link used again; not the reset, the session read, and the two sign-ins
    assert_eq!(diverged, 9, "{d:?}");
    assert!(!d.iter().any(|m| m.contains("exchange \"the reset\":")), "the success has no code to drop: {d:?}");
}

/// A recording that was changed must be seen to differ: the redirect of the second exchange (the uid of no account) is made to say it
/// was refused for its password, and the replay must diverge there and nowhere else.
#[test]
fn parity_control_a_mutated_reset_recording_is_a_divergence() {
    let d = journey(ROWS, "X11", |fixture| {
        fixture["exchanges"][2]["response"]["headers"]["location"] =
            json!("<base_url>/accounts/reset-password?error_code=5020&error_message=INVALID_PASSWORD");
    });
    assert_eq!(d.iter().filter(|m| m.starts_with("exchange ")).count(), 1, "{d:?}");
    assert!(d[0].contains("the uid of no account"), "{d:?}");
}
