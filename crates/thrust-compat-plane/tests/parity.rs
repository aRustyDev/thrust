//! One `parity_` test per compat row: the route answers what the pinned Plane (`oracle/v1-pin-20261018`, `ba85d4d3d3`)
//! recorded, after both are normalised by the same rules (tests/parity/normalise.json). The fixtures are output recorded
//! from that independent oracle by tools/compat-record/record.py; they are never edited by hand (AGENTS.md testing rule
//! 9c). A row's fixture holds every exchange the issue's acceptance criteria name for it, the 401/403/404 ones included.
//!
//! The rows are one journey, in the order of `rows!` below: each test replays the earlier rows on the same world first,
//! so a sign-in finds the user that the sign-up made, as it did on the pin. The sign-in and sign-up rows are compared
//! through the projection of the pin's 302 onto Thrust's JSON (D-022), documented in tests/common/mod.rs.

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
}

/// A fixture without a test would pass unseen, and a test without a fixture is `load_fixture` panicking: hold the two
/// lists equal, so adding a row means adding both.
#[test]
fn parity_every_fixture_has_a_test() {
    let mut rows: Vec<String> = ROWS.iter().map(|r| r.to_string()).collect();
    rows.sort();
    assert_eq!(fixture_rows(), rows, "tests/parity/fixtures/*.json and the rows! list must name the same rows");
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
        let ex = f["exchanges"].as_array_mut().unwrap().iter_mut().find(|e| e["name"] == "sign in with a next_path").unwrap();
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
    let r = redirect("<base_url>/?next_path=/dashboard&error_code=5065&error_message=AUTHENTICATION_FAILED_SIGN_IN&email=");
    assert_eq!(r["response"]["body"], json!({"error_code": 5065, "error_message": "AUTHENTICATION_FAILED_SIGN_IN", "email": ""}));

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
        let got: Vec<Value> = case["input"].as_array().unwrap().iter().map(|ex| normalise::exchange(&rules, &mut ids, ex)).collect();
        assert_eq!(Value::Array(got), case["expected"], "case: {}", case["name"]);
    }
}
