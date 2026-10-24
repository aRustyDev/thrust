//! Plane's internal API (`/api/...`, `/auth/...`) over HTTP, from `thrust-compat-plane` (issue #44). The hermetic
//! `parity_` tests of that crate compare each row with a recording of the pinned Plane; this module holds what has to be
//! true on every deployed target: both composition roots mount the routes, and the status codes the React client acts on
//! (research/0003 section 9, rows 2 and 3).

use serde_json::Value;
use thrust_contract_tests::*;

use super::t;

/// S01. The boot request answers anyone, before there is a session, and says setup is done: a `false` would send the
/// visitor to Plane's instance-admin setup, which Thrust does not have.
#[tokio::test]
async fn compat_instances_answers_without_a_session_and_reports_setup_done() {
    let t = t();
    let r = t.http.get(t.url("/api/instances/")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["instance"]["is_setup_done"], true, "{body}");
    assert_eq!(body["config"]["is_email_password_enabled"], true, "{body}");
}

/// S02. Without a session the answer is a 401 whose body is a JSON object with no `id`: the React client treats any 401
/// as "sign in" and reads the signed-in state from the `id`. Controls: an unknown bearer token is also "no session", and
/// a real session is not refused with a 401, so the 401 is not the route's constant answer.
#[tokio::test]
async fn compat_users_me_without_a_session_is_401_json_with_no_id() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let url = t.url("/api/users/me/");

    for (what, req) in [
        ("no credentials", t.http.get(&url)),
        ("an unknown bearer token", t.http.get(&url).bearer_auth("not-a-session-token")),
    ] {
        let r = req.send().await.unwrap();
        assert_eq!(r.status(), 401, "{what}");
        let body: Value = r.json().await.unwrap();
        assert!(body.is_object(), "{what}: {body}");
        assert!(body.get("id").is_none(), "{what}: a signed-out answer must not carry an id: {body}");
    }

    let r = t.http.get(&url).bearer_auth(&f.admin_a.token).send().await.unwrap();
    // 200 once the boot stage builds the user; until then a 501. Either way, not "sign in".
    assert_ne!(r.status(), 401, "a real session was treated as signed out");
}
