//! The ONE contract suite. Identical assertions and fixtures on every target.
//!
//! Every test that asserts "X is allowed" is paired with a control asserting "the neighbouring Y
//! is rejected" (cross-tenant, non-member, guest, duplicate, over-limit, oversize), so a server
//! that accepts everything cannot pass. `scripts/mutants.sh` additionally runs this suite
//! against deliberately broken servers and requires it to FAIL.
//!
//! The hand-written scenarios below each pin one behaviour. `model` generates them instead: random operation
//! sequences against a reference model, and every caller × project × route against the access table.

mod auth;
mod budget;
mod compat;
mod model;
mod v1_model;

use std::time::Duration;

use futures_util::future::join_all;
use reqwest::Method;
use serde_json::{json, Value};
use thrust_contract_tests::*;
use thrust_contracts::*;

fn t() -> Target {
    Target::from_env()
}

#[tokio::test]
async fn health_ok() {
    let t = t();
    let r = t.http.get(t.url("/healthz")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.unwrap(), "ok");
}

/// The prefix the slice's routes left (#47, FR-06): `/api/v1` is reserved for routes that are wire-compatible with
/// Plane. This is the one place the suite names it, on purpose, to show nothing is served there any more.
const RETIRED_SLICE_PREFIX: &str = "/api/v1";

/// Every slice route as a method and the part after `.../projects/{project_id}`, with `{item}` for a work item id.
/// A route added to `thrust_api::router()` later must be added here.
fn slice_routes() -> [(Method, &'static str); 9] {
    [
        (Method::POST, "/work-items"),
        (Method::GET, "/work-items"),
        (Method::GET, "/work-items/search"),
        (Method::DELETE, "/work-items/{item}"),
        (Method::GET, "/work-items/{item}/activity"),
        (Method::GET, "/activity"),
        (Method::PUT, "/work-items/{item}/attachments/notes.txt"),
        (Method::GET, "/work-items/{item}/attachments/notes.txt"),
        (Method::DELETE, "/work-items/{item}/attachments/notes.txt"),
    ]
}

/// Every slice route that is not under a project: a method and the part after the prefix (issue #38's identity routes).
/// A route added to `thrust_api::router()` later must be added here, too. They are called with no credentials: sign-out
/// and sign-out-everywhere with a real token would end the very sessions the rest of the test uses.
fn slice_auth_routes() -> [(Method, &'static str); 10] {
    [
        (Method::GET, "/auth/oidc/start"),
        (Method::GET, "/auth/oidc/callback"),
        (Method::POST, "/api-keys"),
        (Method::GET, "/api-keys"),
        (Method::DELETE, "/api-keys/00000000-0000-0000-0000-000000000000"),
        (Method::POST, "/auth/sign-in/email"),
        (Method::POST, "/auth/sign-out"),
        (Method::POST, "/auth/sign-out-all"),
        (Method::GET, "/auth/csrf"),
        (Method::GET, "/auth/session"),
    ]
}

/// A 404 from routing: no route matched. A 404 the use-cases raise (an unknown project, workspace or item) carries
/// the API's JSON error body, so a body without an error code is what tells the two apart.
fn is_routing_404(status: u16, body: &[u8]) -> bool {
    let code = serde_json::from_slice::<Value>(body).ok().and_then(|v| v["error"]["code"].as_str().map(str::to_owned));
    status == 404 && code.is_none()
}

/// The slice's routes left `/api/v1`: every one of them is a routing 404 through the retired prefix and is mounted
/// through the live one. The old path with a real token, workspace slug and project is the discriminating case: it
/// answered before the move, so this test fails against the router as it was. (The path from before #36, without the
/// slug, answered 404 already, so it would prove nothing.) The control is the same request through `SLICE_PREFIX`,
/// which is routed, so the 404 above is the prefix and not a bad request. A pinned regression for FR-06; once #58
/// serves `/api/v1` again this test is retired or rewritten with the `_slice` routes.
#[tokio::test]
async fn the_slice_routes_are_gone_from_the_reserved_v1_prefix() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let item = WorkItemId(uuid::Uuid::new_v4());
    for (method, tail) in slice_routes() {
        let tail = tail.replace("{item}", &item.to_string());
        for (prefix, retired) in [(RETIRED_SLICE_PREFIX, true), (SLICE_PREFIX, false)] {
            let url = t.url(&format!("{prefix}/workspaces/{}/projects/{}{tail}", f.a.slug, f.a.project));
            let req = t.http.request(method.clone(), url).bearer_auth(&f.admin_a.token);
            let req = match method {
                Method::POST => req.json(&json!({"name": "x"})),
                Method::PUT => req.body(b"x".to_vec()),
                _ => req,
            };
            let r = req.send().await.unwrap();
            let status = r.status().as_u16();
            let body = r.bytes().await.unwrap();
            let routing_404 = is_routing_404(status, &body);
            if retired {
                assert!(routing_404, "{method} {prefix}...{tail}: got {status}, want a routing 404");
            } else {
                assert!(!routing_404, "{method} {prefix}...{tail}: not mounted ({status})");
            }
        }
    }
    // the identity routes left `/api/v1` as well, and are mounted under the live prefix
    for (method, tail) in slice_auth_routes() {
        for (prefix, retired) in [(RETIRED_SLICE_PREFIX, true), (SLICE_PREFIX, false)] {
            // not following redirects: the live `oidc/start` is a 302 to the identity provider, which is not under test here
            let req = t.no_redirect().request(method.clone(), t.url(&format!("{prefix}{tail}")));
            let req = if method == Method::POST { req.json(&json!({})) } else { req };
            let r = req.send().await.unwrap();
            let status = r.status().as_u16();
            let body = r.bytes().await.unwrap();
            let routing_404 = is_routing_404(status, &body);
            if retired {
                assert!(routing_404, "{method} {prefix}{tail}: got {status}, want a routing 404");
            } else {
                assert!(!routing_404, "{method} {prefix}{tail}: not mounted ({status})");
            }
        }
    }
    // the retired-prefix calls had no effect: only the one create made through the live prefix is there
    let items: Vec<WorkItem> = list(&t, &f.admin_a, &f.a, None).await.json().await.unwrap();
    assert_eq!(items.len(), 1, "a request through the retired prefix reached a use-case");
}

/// `/api/v1` authenticates by `X-Api-Key` and by nothing else, and answers as Plane does: only a missing key is a 401, and a key that
/// is not valid is a 403 with Plane's words (ADR-0031, the issue's "Only a missing key gets 401").
#[tokio::test]
async fn unauthenticated_requests_are_rejected() {
    let t = t();
    let f = Fixture::seed(&t).await;
    // no credentials
    let r = t.http.post(t.url(&v1_items(&f.a))).json(&json!({"name":"x"})).send().await.unwrap();
    assert_eq!(r.status(), 401, "no key");
    // a key that is not one
    let r = t
        .http
        .post(t.url(&v1_items(&f.a)))
        .header(API_KEY_HEADER, "not-a-key")
        .json(&json!({"name":"x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403, "bad key");
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"detail": "Given API token is not valid"}));
    // a session is not a credential here: the route reads no cookie and no bearer token, so a live session token is a missing key
    let r = t
        .http
        .post(t.url(&v1_items(&f.a)))
        .bearer_auth(&f.member_a.token)
        .json(&json!({"name":"x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401, "a session token is no key");
    // control: nothing was created by the rejected calls
    assert!(v1_list_ok(&t, &f.admin_a, &f.a).await.is_empty());
}

#[tokio::test]
async fn internal_routes_require_the_admin_token() {
    let t = t();
    let r = t.http.post(t.url("/internal/sweep")).send().await.unwrap();
    assert_eq!(r.status(), 401, "no admin token");
    let r = t.http.post(t.url("/internal/sweep")).header("x-admin-token", "wrong").send().await.unwrap();
    assert_eq!(r.status(), 401, "wrong admin token");
}

/// `/internal/reset` (issue #58) deletes every row, so this is the one internal route whose gate matters most, and the one this
/// suite cannot call with the token: it runs in parallel against one shared target, and a reset would take the other tests' rows.
/// (The positive path is tested where it cannot hurt anything: the adapters' own tests, and `scripts/oracle-v1-reset.sh`.) A refused
/// call deletes nothing, which is the control: a fixture made before the calls still answers after them.
#[tokio::test]
async fn reset_requires_the_admin_token_and_a_refused_call_deletes_nothing() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let before = v1_list_ok(&t, &f.admin_a, &f.a).await;
    let made = v1_create_ok(&t, &f.member_a, &f.a, "must survive a refused reset").await;

    let r = t.http.post(t.url("/internal/reset")).send().await.unwrap();
    assert_eq!(r.status(), 401, "no admin token");
    let r = t.http.post(t.url("/internal/reset")).header("x-admin-token", "wrong").send().await.unwrap();
    assert_eq!(r.status(), 401, "wrong admin token");
    // only POST: a GET with the right token finds no such route (or refuses the method), and deletes nothing either
    let r = t.admin_get("/internal/reset").await;
    assert!(matches!(r.status().as_u16(), 404 | 405), "a GET is not a reset: {}", r.status());

    let after = v1_list_ok(&t, &f.admin_a, &f.a).await;
    assert_eq!(after.len(), before.len() + 1, "the item made before the refused calls is still there");
    assert!(after.iter().any(|w| w.id() == made.id()));
}

#[tokio::test]
async fn create_allocates_monotonic_sequence_and_sort_order() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let a = v1_create_ok(&t, &f.member_a, &f.a, "first").await;
    let b = v1_create_ok(&t, &f.member_a, &f.a, "second").await;
    assert_eq!((a.sequence_id(), b.sequence_id()), (1, 2));
    // the pin puts the first work item of a state at 65535 and each next one 10000 after the greatest there
    assert_eq!((a.sort_order(), b.sort_order()), (65_535.0, 75_535.0));
    assert_eq!(a.workspace(), Some(f.ws_a));
    assert_eq!(a.created_by(), Some(f.member_a.id));
    assert!(a.0["deleted_at"].is_null());
    // control: a different project has its own counter (sequence is per project, not global)
    let c = v1_create_ok(&t, &f.admin_b, &f.b, "other tenant").await;
    assert_eq!(c.sequence_id(), 1);
}

/// The pin's list is newest first; any member of the project reads it, a guest too.
#[tokio::test]
async fn list_returns_visible_items_newest_first() {
    let t = t();
    let f = Fixture::seed(&t).await;
    for n in ["a", "b", "c"] {
        v1_create_ok(&t, &f.member_a, &f.a, n).await;
    }
    let items = v1_list_ok(&t, &f.guest_a, &f.a).await;
    let names: Vec<_> = items.iter().map(Issue::name).collect();
    assert_eq!(names, ["c", "b", "a"]);
    assert_eq!(items.iter().map(Issue::sequence_id).collect::<Vec<_>>(), [3, 2, 1]);
}

/// A deleted work item is gone from the list and the detail, and its number is never used again (ADR-0043 B1). The pin lets a project
/// admin delete any work item and a member only one they made, and a guest none; there is no view of a deleted row in `/api/v1`.
#[tokio::test]
async fn soft_delete_hides_rows_and_never_reuses_sequence_ids() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let one = v1_create_ok(&t, &f.member_a, &f.a, "one").await;
    let two = v1_create_ok(&t, &f.member_a, &f.a, "two").await;
    let _three = v1_create_ok(&t, &f.member_a, &f.a, "three").await;
    let admins = v1_create_ok(&t, &f.admin_a, &f.a, "the admin's").await;

    // control 1: a guest may not delete, and a member may not delete what the admin made; neither is deleted by being asked
    assert_eq!(v1_delete(&t, &f.guest_a, &f.a, &one.id()).await.status(), 403, "a guest deletes nothing");
    let r = v1_delete(&t, &f.member_a, &f.a, &admins.id()).await;
    assert_eq!(r.status(), 403, "a member deletes only what they made");
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"error": "Only admin or creator can delete the work item"}));
    assert_eq!(v1_list_ok(&t, &f.member_a, &f.a).await.len(), 4, "refused deletions deleted nothing");

    // a member deletes one they made
    assert_eq!(v1_delete(&t, &f.member_a, &f.a, &two.id()).await.status(), 204);
    // the list hides it, newest first
    let visible = v1_list_ok(&t, &f.member_a, &f.a).await;
    assert_eq!(visible.iter().map(Issue::sequence_id).collect::<Vec<_>>(), [4, 3, 1]);
    // and so does the detail: it is a 404 now
    let r = t.http.get(t.url(&v1_item(&f.a, &two.id()))).header(API_KEY_HEADER, &f.member_a.key).send().await.unwrap();
    assert_eq!(r.status(), 404, "the detail of a deleted work item");

    // an admin deletes the one the member could not
    assert_eq!(v1_delete(&t, &f.admin_a, &f.a, &admins.id()).await.status(), 204);

    // deleting twice is a 404, not a second success
    assert_eq!(v1_delete(&t, &f.member_a, &f.a, &two.id()).await.status(), 404);

    // numbers are never reused: the next item is #5, not #2 or #4
    let five = v1_create_ok(&t, &f.member_a, &f.a, "five").await;
    assert_eq!(five.sequence_id(), 5);
}

/// Tenant B's admin reaches for tenant A's project and is refused, and nothing is made. The pin refuses a stranger with the 403 it gives
/// anyone who is not a member of the project, and answers a project that is not there the same way (`ProjectEntityPermission`), so
/// every one of the cases below is a 403: what tells them apart from a bad route is that tenant A's own admin is not refused.
#[tokio::test]
async fn cross_tenant_access_is_rejected_and_has_no_effect() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let r = v1_create(&t, &f.admin_b, &f.a, json!({"name": "intruder"})).await;
    assert_eq!(r.status(), 403, "cross-tenant create");
    assert_eq!(v1_list(&t, &f.admin_b, &f.a).await.status(), 403, "cross-tenant list");
    // control: the rejected create left nothing behind in tenant A ...
    assert!(v1_list_ok(&t, &f.admin_a, &f.a).await.is_empty(), "cross-tenant create leaked a row");
    // ... and tenant A's own admin is NOT refused on the same path (the 403 above is tenancy, not a bad route)
    assert_eq!(v1_list(&t, &f.admin_a, &f.a).await.status(), 200);
    // The other URL form: tenant B's own slug with tenant A's project id. The project is not in the workspace the slug names, so it is
    // refused as well, and so is a slug that names no workspace at all.
    assert_eq!(v1_list(&t, &f.admin_b, &f.a.via(&f.b.slug)).await.status(), 403, "A's project under B's slug");
    assert_eq!(v1_list(&t, &f.admin_a, &f.a.via("no-such-workspace")).await.status(), 403, "unknown slug");
    // a workspace member who is not a member of the project is refused in the same words
    assert_eq!(v1_list(&t, &f.outsider_a, &f.a).await.status(), 403);
}

/// The session row is read on every request (ADR-0052 round 29), so a revocation is visible to the very next request,
/// on every target: there is no cache to wait out. The revocation goes through the test route until sign-out exists
/// (#38).
#[tokio::test]
async fn a_revoked_session_is_rejected_on_the_next_request() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let other = f.extra_member(&t).await;
    assert_eq!(list(&t, &f.member_a, &f.a, None).await.status(), 200, "before the revoke");
    t.revoke_session(&f.member_a.token).await;
    // the next request, and every kind of route, not only the one used first
    assert_eq!(list(&t, &f.member_a, &f.a, None).await.status(), 401, "list with the revoked token");
    assert_eq!(create(&t, &f.member_a, &f.a, json!({"name": "x"})).await.status(), 401, "create with it");
    // control: an unrevoked token on the same request still gets 200, so the 401 is the revoke, not the route
    assert_eq!(list(&t, &other, &f.a, None).await.status(), 200, "an unrevoked token");
    assert_eq!(list(&t, &f.admin_a, &f.a, None).await.status(), 200, "another unrevoked token");
    // revoking again, or revoking a token that was never issued, changes nothing
    t.revoke_session(&f.member_a.token).await;
    t.revoke_session("never-issued").await;
    assert_eq!(list(&t, &other, &f.a, None).await.status(), 200, "still unrevoked after the no-op revokes");
}

/// A session belongs to a user, not to a workspace (ADR-0052 round 29): `dual` belongs to both workspaces and its one
/// token works in each, with that workspace's project.
#[tokio::test]
async fn a_session_is_not_bound_to_a_workspace() {
    let t = t();
    let f = Fixture::seed(&t).await;
    assert_eq!(list(&t, &f.dual, &f.a, None).await.status(), 200, "dual in workspace A");
    assert_eq!(list(&t, &f.dual, &f.b, None).await.status(), 200, "dual in workspace B");
    // the two workspaces stay two tenants: what dual creates in A is not in B
    let item = create_ok(&t, &f.dual, &f.a, "from dual in A").await;
    let in_b: Vec<WorkItem> = list(&t, &f.dual, &f.b, None).await.json().await.unwrap();
    assert!(in_b.iter().all(|w| w.id != item.id), "an item of workspace A showed up in workspace B");
    // control: a token of a user who belongs to one workspace only gets 404 on a project of the other
    assert_eq!(list(&t, &f.admin_a, &f.b, None).await.status(), 404, "admin_a is not in workspace B");
    assert_eq!(list(&t, &f.admin_b, &f.a, None).await.status(), 404, "admin_b is not in workspace A");
}

#[tokio::test]
async fn non_members_and_guests_are_forbidden_to_write() {
    let t = t();
    let f = Fixture::seed(&t).await;
    assert_eq!(v1_create(&t, &f.outsider_a, &f.a, json!({"name":"x"})).await.status(), 403, "outsider create");
    assert_eq!(v1_list(&t, &f.outsider_a, &f.a).await.status(), 403, "outsider list");
    assert_eq!(v1_create(&t, &f.guest_a, &f.a, json!({"name":"x"})).await.status(), 403, "guest create");
    // control: guests can read, members can write
    assert_eq!(v1_list(&t, &f.guest_a, &f.a).await.status(), 200, "guest list");
    assert_eq!(v1_create(&t, &f.member_a, &f.a, json!({"name":"ok"})).await.status(), 201);
    // and the refusals made nothing
    assert_eq!(v1_list_ok(&t, &f.admin_a, &f.a).await.len(), 1);
}

/// The key is the pair (source, id): the same pair twice is the pin's 409 that names the work item that has it, and a different id, a
/// different source, or one half alone is not a clash.
#[tokio::test]
async fn external_id_is_unique_among_live_rows_only() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let body = |ext: &str| json!({"name": "imported", "external_source": "github", "external_id": ext});
    let first = v1_create(&t, &f.member_a, &f.a, body("42")).await;
    assert_eq!(first.status(), 201);
    let first = Issue(first.json().await.unwrap());
    // duplicate live (source, id) -> 409 (the must-fail control for the unique constraint), naming the work item that has it
    let dup = v1_create(&t, &f.member_a, &f.a, body("42")).await;
    assert_eq!(dup.status(), 409, "duplicate external_id must conflict");
    assert_eq!(
        dup.json::<Value>().await.unwrap(),
        json!({"error": "Issue with the same external id and external source already exists", "id": first.id()})
    );
    // control: a different id is fine
    assert_eq!(v1_create(&t, &f.member_a, &f.a, body("43")).await.status(), 201);
    // the failed duplicate must not have burned a visible row
    assert_eq!(v1_list_ok(&t, &f.member_a, &f.a).await.len(), 2);
    // after a soft delete the key is free again (partial unique index semantics)
    assert_eq!(v1_delete(&t, &f.member_a, &f.a, &first.id()).await.status(), 204);
    assert_eq!(v1_create(&t, &f.member_a, &f.a, body("42")).await.status(), 201, "re-create after soft delete");
}

/// Twenty creates at once get twenty distinct numbers, with none missing. (The pin reads the greatest `sort_order` of a state before it
/// inserts, so two creates at once can share a place in it; that is the pin's, and is not asserted.)
#[tokio::test]
async fn concurrent_creates_never_duplicate_a_sequence_id() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let n = 20usize;
    let futs = (0..n).map(|i| {
        let t = &t;
        let f = &f;
        async move { v1_create(t, &f.member_a, &f.a, json!({"name": format!("c{i}")})).await }
    });
    let resps = join_all(futs).await;
    let mut seqs = Vec::new();
    for r in resps {
        assert_eq!(r.status(), 201);
        seqs.push(Issue(r.json().await.unwrap()).sequence_id());
    }
    seqs.sort();
    assert_eq!(seqs, (1..=n as i64).collect::<Vec<_>>(), "sequence ids must be unique and contiguous");
    assert_eq!(v1_list_ok(&t, &f.admin_a, &f.a).await.len(), n, "and every create is a work item of the list");
}

#[tokio::test]
async fn outbox_event_reaches_the_queue_consumer_exactly_once() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let item = create_ok(&t, &f.member_a, &f.a, "evented").await;

    let acts = eventually("created activity row", Duration::from_secs(15), || async {
        let a = activity(&t, &f.guest_a, &f.a, item.id).await;
        (!a.is_empty()).then_some(a)
    })
    .await;
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0].verb, "created");
    assert_eq!(acts[0].actor_id, Some(f.member_a.id));
    assert_eq!(acts[0].entity_id, item.id.as_uuid());
    assert_eq!(acts[0].workspace_id, f.ws_a);

    // delete -> second event -> second activity row; no duplicates of the first
    let r = t
        .http
        .delete(t.url(&format!("{}/{}", wi_path(&f.a), item.id)))
        .bearer_auth(&f.member_a.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let acts = eventually("deleted activity row", Duration::from_secs(15), || async {
        let a = activity(&t, &f.guest_a, &f.a, item.id).await;
        (a.len() >= 2).then_some(a)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let acts_later = activity(&t, &f.guest_a, &f.a, item.id).await;
    assert_eq!(acts.len(), 2);
    assert_eq!(acts_later.len(), 2, "redelivery must not duplicate activity rows");
    let mut verbs: Vec<_> = acts_later.iter().map(|a| a.verb.clone()).collect();
    verbs.sort();
    assert_eq!(verbs, ["created", "deleted"]);

    // control: tenant B cannot read tenant A's activity
    let r = t
        .http
        .get(t.url(&format!("{}/{}/activity", wi_path(&f.a), item.id)))
        .bearer_auth(&f.admin_b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

/// The test above never actually redelivers anything, so on its own it cannot tell an idempotent
/// consumer from a consumer that happens to be called once. This one forces at-least-once
/// behaviour: the SAME envelope is put on the queue twice, and a different one once.
#[tokio::test]
async fn redelivered_event_is_processed_once() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let item = create_ok(&t, &f.member_a, &f.a, "redelivery").await;
    eventually("original activity row", Duration::from_secs(15), || async {
        (activity(&t, &f.guest_a, &f.a, item.id).await.len() == 1).then_some(())
    })
    .await;

    let envelope = |event_id: uuid::Uuid, verb: &str| {
        json!({
            "event_id": event_id,
            "schema_version": 1,
            "workspace_id": f.ws_a,
            "event_type": format!("work_item.{verb}"),
            "entity_type": "work_item",
            "entity_id": item.id,
            "actor_id": f.member_a.id,
            "occurred_at": "2026-10-18T00:00:00Z",
            "payload": { "project_id": f.a.project, "changed_fields": ["name"] },
            "trace_id": null
        })
    };
    let dup_id = uuid::Uuid::new_v4();
    for _ in 0..2 {
        let r = t.admin_post("/internal/inject-event", &envelope(dup_id, "updated")).await;
        assert_eq!(r.status(), 202, "inject: {:?}", r.text().await);
    }
    // control: a DIFFERENT event id must still be processed (the dedupe is by event id, not "drop everything")
    let r = t.admin_post("/internal/inject-event", &envelope(uuid::Uuid::new_v4(), "moved")).await;
    assert_eq!(r.status(), 202);

    eventually("both distinct injected events", Duration::from_secs(15), || async {
        (activity(&t, &f.guest_a, &f.a, item.id).await.len() >= 3).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(2000)).await; // give a (wrong) duplicate time to show up
    let acts = activity(&t, &f.guest_a, &f.a, item.id).await;
    let mut verbs: Vec<_> = acts.iter().map(|a| a.verb.clone()).collect();
    verbs.sort();
    assert_eq!(verbs, ["created", "moved", "updated"], "the duplicated event must produce ONE activity row");
}

/// What every target's create limit has in common (issue #43, D-007). The limit is exact on the native server and on the
/// in-isolate Worker, and approximate on the Cloudflare Worker, where it is a Workers Rate Limiting binding that counts per
/// location (a binding of 3 has let 4 through). So the test holds only what is true of all of them:
///
/// * the first `limit` creates of a user pass;
/// * a 429 with a `Retry-After` of at least a second comes within `2 x limit + 1` creates. A fixed window aligned to the
///   clock lets a burst that straddles a boundary have up to twice the limit, and an approximate one a little more, so
///   this is the latest a refusal can be owed however the burst falls on the windows, and the test needs no clock;
/// * the limit is per user: another member's create is not refused.
///
/// It used to assert the exact rule (`limit` creates pass and the next one is refused, repeating a burst that crossed a
/// window boundary), which the binding does not promise. The exact rule is held where a limiter does promise it, by the
/// `prop_` tests of `MemoryRateLimiter` in `thrust-adapters-memory`. A limit that is never reached (the binding raised
/// above `2 x limit + 1`) fails this test: that is its control.
#[tokio::test]
async fn create_is_rate_limited_per_user() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let limit = Target::create_limit();
    let user = f.extra_member(&t).await;
    for i in 0..limit {
        let r = create(&t, &user, &f.a, json!({"name": format!("r{i}")})).await;
        assert_eq!(r.status(), 201, "request {i} of {limit} must pass");
    }
    let mut refused = None;
    for i in limit..=(2 * limit) {
        let r = create(&t, &user, &f.a, json!({"name": format!("r{i}")})).await;
        if r.status() == 429 {
            refused = Some(r);
            break;
        }
        // anything but a pass or a refusal (a 500 from the limiter, say) is not "not limited yet"
        assert_eq!(r.status(), 201, "request {i}: not limited yet, so it must be created");
    }
    let r =
        refused.unwrap_or_else(|| panic!("none of the first {} creates was limited (limit {limit})", 2 * limit + 1));
    let retry_after = r.headers().get("retry-after").expect("a 429 carries Retry-After");
    let secs: u64 = retry_after.to_str().unwrap().parse().expect("Retry-After is a number of seconds");
    assert!(secs >= 1, "Retry-After must be at least a second, not {secs}");
    // control: the limit is per user, another member is unaffected
    let other = f.extra_member(&t).await;
    assert_eq!(create(&t, &other, &f.a, json!({"name": "fine"})).await.status(), 201);
}

#[tokio::test]
async fn attachments_roundtrip_with_authz() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let item = create_ok(&t, &f.member_a, &f.a, "with file").await;
    let path = format!("{}/{}/attachments/notes.txt", wi_path(&f.a), item.id);
    let payload = b"hello r2 \x00\x01\x02 binary-safe".to_vec();

    // guest cannot write (control)
    let r = t.http.put(t.url(&path)).bearer_auth(&f.guest_a.token).body(payload.clone()).send().await.unwrap();
    assert_eq!(r.status(), 403);
    // member can
    let r = t
        .http
        .put(t.url(&path))
        .bearer_auth(&f.member_a.token)
        .header("content-type", "application/x-thrust")
        .body(payload.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    // guest can read, bytes and content-type survive
    let r = t.http.get(t.url(&path)).bearer_auth(&f.guest_a.token).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers().get("content-type").unwrap(), "application/x-thrust");
    assert_eq!(r.bytes().await.unwrap().to_vec(), payload);
    // unknown name -> 404 (control: the 200 above is not "always 200")
    let other = format!("{}/{}/attachments/missing.txt", wi_path(&f.a), item.id);
    assert_eq!(t.http.get(t.url(&other)).bearer_auth(&f.guest_a.token).send().await.unwrap().status(), 404);
    // other tenant cannot read it
    assert_eq!(t.http.get(t.url(&path)).bearer_auth(&f.admin_b.token).send().await.unwrap().status(), 404);
    // path traversal in the name is rejected
    let evil = format!("{}/{}/attachments/..%2Fescape", wi_path(&f.a), item.id);
    let st = t.http.put(t.url(&evil)).bearer_auth(&f.member_a.token).body("x").send().await.unwrap().status();
    assert!(st == 400 || st == 404, "traversal name must not be accepted, got {st}");
    // oversize is rejected before touching storage (limit is 5 MiB)
    let big = vec![7u8; 5 * 1024 * 1024 + 1];
    let r = t.http.put(t.url(&path)).bearer_auth(&f.member_a.token).body(big).send().await.unwrap();
    assert_eq!(r.status(), 413);
    // the oversize attempt did not clobber the stored object
    let r = t.http.get(t.url(&path)).bearer_auth(&f.guest_a.token).send().await.unwrap();
    assert_eq!(r.bytes().await.unwrap().to_vec(), payload);
    // delete, then it is gone
    let r = t.http.delete(t.url(&path)).bearer_auth(&f.member_a.token).send().await.unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(t.http.get(t.url(&path)).bearer_auth(&f.guest_a.token).send().await.unwrap().status(), 404);
}

/// A body that is wrong is a 400 in DRF's words, field by field.
#[tokio::test]
async fn invalid_input_is_a_client_error() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let r = v1_create(&t, &f.member_a, &f.a, json!({"name": "   "})).await;
    assert_eq!(r.status(), 400);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"name": ["This field may not be blank."]}));
    let long = "x".repeat(256);
    let r = v1_create(&t, &f.member_a, &f.a, json!({"name": long})).await;
    assert_eq!(r.status(), 400);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"name": ["Ensure this field has no more than 255 characters."]})
    );
    // not a uuid in the path
    let r = t
        .http
        .post(t.url(&format!("{V1_PREFIX}/workspaces/{}/projects/not-a-uuid/issues/", f.a.slug)))
        .header(API_KEY_HEADER, &f.member_a.key)
        .json(&json!({"name":"x"}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_client_error(), "got {}", r.status());
    // control: a 255-char name is accepted
    assert_eq!(v1_create(&t, &f.member_a, &f.a, json!({"name": "y".repeat(255)})).await.status(), 201);
    // and the refused bodies made nothing
    assert_eq!(v1_list_ok(&t, &f.admin_a, &f.a).await.len(), 1);
}

/// Own pass (`panic_`): a panic under panic=abort resets the whole Worker isolate, which would
/// kill every other test running in parallel (measured: it did). Run alone, against a fresh
/// target.
///
/// `THRUST_STATE_SURVIVES_PANIC=0` is set only for targets whose state lives inside the isolate
/// (worker-memory); there losing state is the documented consequence. Everywhere else the
/// pre-panic row MUST still be there.
#[tokio::test]
async fn panic_is_contained_and_the_target_keeps_serving() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let before = create_ok(&t, &f.member_a, &f.a, "before-panic").await;

    match t.http.get(t.url("/_test/panic")).send().await {
        Ok(r) => {
            assert!(r.status().is_server_error(), "panic route must answer 5xx, got {}", r.status());
            eprintln!("PANIC-OBSERVATION: the panicking request got HTTP {}", r.status());
        }
        Err(e) => eprintln!("PANIC-OBSERVATION: the panicking request got a transport error: {e}"),
    }

    // control: the target serves again (a Worker re-instantiates lazily on the next request)
    let mut ok = false;
    for _ in 0..10 {
        if let Ok(r) = t.http.get(t.url("/healthz")).send().await {
            if r.status() == 200 {
                ok = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(ok, "target did not recover after a panic");

    let survives = std::env::var("THRUST_STATE_SURVIVES_PANIC").map(|v| v != "0").unwrap_or(true);
    let r = list(&t, &f.member_a, &f.a, None).await;
    if survives {
        assert_eq!(r.status(), 200, "durable state must survive a panic");
        let items: Vec<WorkItem> = r.json().await.unwrap();
        assert!(items.iter().any(|i| i.id == before.id), "row written before the panic is gone");
    } else {
        eprintln!(
            "PANIC-OBSERVATION: in-isolate state after the panic: HTTP {} (loss is expected for this target)",
            r.status()
        );
    }
}

/// Blast radius of a panic. 16 clients hammer an AUTHENTICATED list request (it awaits database /
/// KV I/O, so requests are genuinely suspended mid-flight on Postgres-backed targets) while ONE
/// request panics the handler; we count how many of the OTHER requests fail. Native (panic caught
/// per request): none. Worker (panic=abort, whole isolate reset): suspended requests can fail,
/// and on the in-isolate memory set all state is lost (401s afterwards).
/// The numbers are printed as observations; the assertion is that the target recovers.
#[tokio::test]
async fn blast_radius_of_one_panic() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    let t = Arc::new(t());
    let f = Arc::new(Fixture::seed(&t).await);
    let stop = Arc::new(AtomicBool::new(false));
    let ok = Arc::new(AtomicUsize::new(0));
    let bad = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let (t, f, stop, ok, bad) = (t.clone(), f.clone(), stop.clone(), ok.clone(), bad.clone());
        tasks.push(tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                let r = t.http.get(t.url(&wi_path(&f.a))).bearer_auth(&f.member_a.token).send().await;
                match r {
                    Ok(r) if r.status() == 200 => ok.fetch_add(1, Ordering::Relaxed),
                    _ => bad.fetch_add(1, Ordering::Relaxed),
                };
            }
        }));
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = t.http.get(t.url("/_test/panic")).send().await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    stop.store(true, Ordering::Relaxed);
    for h in tasks {
        let _ = h.await;
    }
    eprintln!(
        "PANIC-OBSERVATION: concurrent authenticated list requests around one panic: ok={} failed={}",
        ok.load(Ordering::Relaxed),
        bad.load(Ordering::Relaxed)
    );
    assert!(ok.load(Ordering::Relaxed) > 100, "the load generator did not actually run");
    // control: the target answers afterwards
    let mut served = false;
    for _ in 0..10 {
        if let Ok(r) = t.http.get(t.url("/healthz")).send().await {
            if r.status() == 200 {
                served = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(served, "target did not recover");
}

/// Only run in the "sweeper" pass, against a target started with the outbox fast path OFF.
#[tokio::test]
async fn sweeper_publishes_rows_the_fast_path_did_not() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let before = t.outbox_depth().await;
    let item = create_ok(&t, &f.member_a, &f.a, "swept").await;
    assert_eq!(t.outbox_depth().await, before + 1, "row must sit in the outbox");
    // control: with the fast path off nothing may have delivered the event yet
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        activity(&t, &f.guest_a, &f.a, item.id).await.is_empty(),
        "an activity row appeared without the sweeper: the fast path is not actually off, this test would be vacuous"
    );
    // run the sweeper (what cron does)
    let r = t.admin_post("/internal/sweep", &json!({})).await;
    assert_eq!(r.status(), 200);
    let rep: Value = r.json().await.unwrap();
    assert!(rep["published"].as_u64().unwrap() >= 1, "sweeper published nothing: {rep}");
    let acts = eventually("activity after sweep", Duration::from_secs(15), || async {
        let a = activity(&t, &f.guest_a, &f.a, item.id).await;
        (!a.is_empty()).then_some(a)
    })
    .await;
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0].verb, "created");
    assert_eq!(t.outbox_depth().await, 0, "swept rows must be marked dispatched");
}
