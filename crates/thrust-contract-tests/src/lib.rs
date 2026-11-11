//! Shared fixtures + HTTP client for the contract suite and the bench binary.
//!
//! The suite (`tests/contract.rs`) is target-agnostic: it only ever sees `THRUST_BASE_URL`.
//! Fixtures are created through `POST /internal/seed`, the same route on every target.

pub mod mock_idp;

use std::time::Duration;

use serde_json::{json, Value};
use thrust_contracts::*;

pub struct Target {
    pub base: String,
    pub admin_token: String,
    pub http: reqwest::Client,
}

impl Target {
    pub fn from_env() -> Target {
        let base = std::env::var("THRUST_BASE_URL").expect("THRUST_BASE_URL must be set (see scripts/contract.sh)");
        let admin_token = std::env::var("THRUST_ADMIN_TOKEN").expect("THRUST_ADMIN_TOKEN must be set");
        let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).no_proxy().build().unwrap();
        Target { base: base.trim_end_matches('/').to_string(), admin_token, http }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// A client that does not follow redirects: the OIDC legs are redirects, and the suite plays the browser itself.
    pub fn no_redirect(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

    /// The create limit the target was started with: `THRUST_CREATE_RATE_LIMIT` on the native server and the in-isolate Worker, and
    /// the `RL_CREATE` binding's limit on the Cloudflare Worker (`scripts/worker-dev.sh` writes it into the config it starts from).
    pub fn create_limit() -> u32 {
        std::env::var("THRUST_EXPECT_CREATE_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(30)
    }

    pub async fn admin_post(&self, path: &str, body: &Value) -> reqwest::Response {
        self.http.post(self.url(path)).header("x-admin-token", &self.admin_token).json(body).send().await.unwrap()
    }

    pub async fn admin_get(&self, path: &str) -> reqwest::Response {
        self.http.get(self.url(path)).header("x-admin-token", &self.admin_token).send().await.unwrap()
    }

    /// Revoke the session behind `token` through the test route. 204 whether or not the token is a session.
    pub async fn revoke_session(&self, token: &str) {
        let r = self.admin_post("/internal/revoke-session", &json!({ "token": token })).await;
        assert_eq!(r.status(), 204, "revoke-session");
    }

    /// What the target says it is (`/_info`): `target`, `adapters` and `csrf_enforce`.
    pub async fn info(&self) -> Value {
        let r = self.admin_get("/_info").await;
        assert_eq!(r.status(), 200, "/_info");
        r.json().await.unwrap()
    }

    pub async fn outbox_depth(&self) -> u64 {
        let r = self.admin_get("/internal/outbox-depth").await;
        assert_eq!(r.status(), 200, "outbox-depth");
        r.json::<Value>().await.unwrap()["undispatched"].as_u64().unwrap()
    }
}

fn uid() -> uuid::Uuid {
    uuid::Uuid::new_v4()
}

/// The label of the key that the fixture plants for every user. A test of the slice's key listing counts the keys it made and not
/// this one (`GET /api-keys` shows it like any other).
pub const FIXTURE_KEY_LABEL: &str = "contract";

/// A seeded user: a session token the fixture planted (the legacy bearer credential of the slice's tests), an API key the
/// fixture planted too (what `/api/v1` authenticates by), and an email and password that sign in through the real route.
#[derive(Clone)]
pub struct User {
    pub id: UserId,
    pub token: String,
    /// The `X-Api-Key` of `/api/v1`. It is shown here because the fixture chose it; a key made through the slice's `/api-keys` is
    /// a [`CreatedApiKey`] instead.
    pub key: String,
    pub email: String,
    pub password: String,
}

impl User {
    fn new() -> User {
        let n = uid().simple();
        User {
            id: UserId(uid()),
            token: format!("tok-{}", uid().simple()),
            key: format!("thrk_{}{}", uid().simple(), uid().simple()),
            email: format!("user-{n}@example.test"),
            password: format!("pw-{}", uid().simple()),
        }
    }

    fn api_key(&self) -> SeedApiKey {
        SeedApiKey {
            token: self.key.clone(),
            user_id: self.id,
            label: FIXTURE_KEY_LABEL.into(),
            expires_at: None,
            revoked: false,
        }
    }

    fn login(&self) -> SeedLogin {
        SeedLogin {
            user_id: self.id,
            email: self.email.clone(),
            password: Some(self.password.clone()),
            is_active: true,
        }
    }
}

/// Names the API uses on the wire; must equal `thrust_api::SESSION_COOKIE`, `CSRF_COOKIE` and `CSRF_HEADER` (this crate
/// is black-box and does not depend on it). A mismatch makes every cookie test fail.
pub const SESSION_COOKIE: &str = "__Host-thrust_session";
pub const CSRF_COOKIE: &str = "__Host-thrust_csrf";
pub const CSRF_HEADER: &str = "x-csrftoken";

/// One `Set-Cookie` header: its name, value and the attributes after them, as sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetCookie {
    pub name: String,
    pub value: String,
    pub attrs: Vec<String>,
}

impl SetCookie {
    /// Every `Set-Cookie` header of a response, in order.
    pub fn all(r: &reqwest::Response) -> Vec<SetCookie> {
        r.headers()
            .get_all("set-cookie")
            .iter()
            .map(|h| {
                let line = h.to_str().expect("set-cookie is ascii");
                let mut parts = line.split(';').map(str::trim);
                let (name, value) = parts.next().and_then(|nv| nv.split_once('=')).expect("name=value");
                SetCookie { name: name.into(), value: value.into(), attrs: parts.map(str::to_string).collect() }
            })
            .collect()
    }

    pub fn find<'a>(cookies: &'a [SetCookie], name: &str) -> Option<&'a SetCookie> {
        cookies.iter().find(|c| c.name == name)
    }

    pub fn has_attr(&self, attr: &str) -> bool {
        self.attrs.iter().any(|a| a.eq_ignore_ascii_case(attr))
    }

    /// The value of an attribute written `Name=value` (`Max-Age`, `SameSite`, `Path`).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find_map(|a| a.split_once('=').filter(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v))
    }
}

/// A signed-in session as a client holds it: the two cookie values from a sign-in response. The suite sends them back
/// itself, because a client cookie jar would withhold a `Secure` cookie over the plain HTTP the local targets serve.
#[derive(Debug, Clone)]
pub struct Session {
    pub user_id: UserId,
    pub token: String,
    pub csrf: String,
}

impl Session {
    /// The `Cookie` header a browser would send.
    pub fn cookie_header(&self) -> String {
        format!("{SESSION_COOKIE}={}; {CSRF_COOKIE}={}", self.token, self.csrf)
    }

    /// The same session with only the session cookie (no CSRF cookie).
    pub fn session_cookie_only(&self) -> String {
        format!("{SESSION_COOKIE}={}", self.token)
    }
}

impl Target {
    pub async fn sign_in_raw(&self, email: &str, password: &str) -> reqwest::Response {
        self.http
            .post(self.url(&format!("{SLICE_PREFIX}/auth/sign-in/email")))
            .json(&json!({ "email": email, "password": password }))
            .send()
            .await
            .unwrap()
    }

    /// Sign in through the real route and keep what the response set. Panics unless it is a 200 with both cookies.
    pub async fn sign_in(&self, who: &User) -> Session {
        let r = self.sign_in_raw(&who.email, &who.password).await;
        assert_eq!(r.status(), 200, "sign-in of a seeded user");
        let cookies = SetCookie::all(&r);
        let body: SignInResponse = r.json().await.unwrap();
        let token = SetCookie::find(&cookies, SESSION_COOKIE).expect("a session cookie").value.clone();
        let csrf = SetCookie::find(&cookies, CSRF_COOKIE).expect("a csrf cookie").value.clone();
        assert_eq!(body.csrf_token, csrf, "the body's csrf token is the cookie's");
        Session { user_id: body.user_id, token, csrf }
    }

    /// Deactivate or reactivate a user through the test route.
    pub async fn set_user_active(&self, user: UserId, active: bool) {
        let r = self.admin_post("/internal/set-user-active", &json!({ "user_id": user, "active": active })).await;
        assert_eq!(r.status(), 204, "set-user-active");
    }
}

/// Where the slice's tenant routes are mounted. It must equal `thrust_api::SLICE_PREFIX` (this crate is black-box and
/// does not depend on it). The slice left `/api/v1` in #47; the one other place the suite names a prefix is
/// `RETIRED_SLICE_PREFIX` in `tests/contract.rs`, which shows the old one is gone.
pub const SLICE_PREFIX: &str = "/api/_slice";

/// A project addressed through a workspace slug: the two segments a slice route carries after the prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct At {
    pub slug: String,
    pub project: ProjectId,
}

impl At {
    /// The same project, addressed through another workspace's slug.
    pub fn via(&self, slug: &str) -> At {
        At { slug: slug.to_string(), project: self.project }
    }
}

/// Two tenants. Tenant A has one project with an admin, a member, a guest and an outsider
/// (same workspace, no membership). Tenant B has its own project and admin. `dual` belongs to both workspaces
/// and is a member of both projects: one session, two workspaces.
pub struct Fixture {
    pub ws_a: WorkspaceId,
    pub ws_b: WorkspaceId,
    /// Project A through workspace A's slug, and project B through workspace B's.
    pub a: At,
    pub b: At,
    pub admin_a: User,
    pub member_a: User,
    pub guest_a: User,
    pub outsider_a: User,
    pub admin_b: User,
    pub dual: User,
}

impl Fixture {
    pub async fn seed(t: &Target) -> Fixture {
        let mk = User::new;
        let ws_a = WorkspaceId(uid());
        let ws_b = WorkspaceId(uid());
        // Slugs are unique in the database, and one database serves every fixture of a run.
        let a = At { slug: format!("ws-a-{}", uid().simple()), project: ProjectId(uid()) };
        let b = At { slug: format!("ws-b-{}", uid().simple()), project: ProjectId(uid()) };
        let (admin_a, member_a, guest_a, outsider_a, admin_b, dual) = (mk(), mk(), mk(), mk(), mk(), mk());

        let in_ws = |workspace_id, u: &User| SeedWorkspaceMember { workspace_id, user_id: u.id };
        let on_a = |u: &User, role| SeedMembership { project_id: a.project, workspace_id: ws_a, user_id: u.id, role };
        let on_b = |u: &User, role| SeedMembership { project_id: b.project, workspace_id: ws_b, user_id: u.id, role };
        let all = [&admin_a, &member_a, &guest_a, &outsider_a, &admin_b, &dual];
        let fixture = SeedFixture {
            workspaces: vec![
                SeedWorkspace { id: ws_a, slug: WorkspaceSlug::new(&a.slug) },
                SeedWorkspace { id: ws_b, slug: WorkspaceSlug::new(&b.slug) },
            ],
            users: all.iter().map(|u| u.id).collect(),
            workspace_members: vec![
                in_ws(ws_a, &admin_a),
                in_ws(ws_a, &member_a),
                in_ws(ws_a, &guest_a),
                in_ws(ws_a, &outsider_a),
                in_ws(ws_b, &admin_b),
                in_ws(ws_a, &dual),
                in_ws(ws_b, &dual),
            ],
            projects: vec![
                SeedProject { id: a.project, workspace_id: ws_a, name: "A".into() },
                SeedProject { id: b.project, workspace_id: ws_b, name: "B".into() },
            ],
            memberships: vec![
                on_a(&admin_a, Role::Admin),
                on_a(&member_a, Role::Member),
                on_a(&guest_a, Role::Guest),
                on_b(&admin_b, Role::Admin),
                on_a(&dual, Role::Member),
                on_b(&dual, Role::Guest),
            ],
            sessions: all.iter().map(|u| SeedSession { token: u.token.clone(), user_id: u.id }).collect(),
            logins: all.iter().map(|u| u.login()).collect(),
            api_keys: all.iter().map(|u| u.api_key()).collect(),
            ..SeedFixture::default()
        };
        let r = t.admin_post("/internal/seed", &serde_json::to_value(&fixture).unwrap()).await;
        assert_eq!(r.status(), 204, "seed failed: {:?}", r.text().await);
        Fixture { ws_a, ws_b, a, b, admin_a, member_a, guest_a, outsider_a, admin_b, dual }
    }

    /// A fresh member of project A with its own rate-limit bucket.
    pub async fn extra_member(&self, t: &Target) -> User {
        let who = User::new();
        let (id, token) = (who.id, who.token.clone());
        let f = SeedFixture {
            workspaces: vec![SeedWorkspace { id: self.ws_a, slug: WorkspaceSlug::new(&self.a.slug) }],
            users: vec![id],
            workspace_members: vec![SeedWorkspaceMember { workspace_id: self.ws_a, user_id: id }],
            projects: vec![],
            memberships: vec![SeedMembership {
                project_id: self.a.project,
                workspace_id: self.ws_a,
                user_id: id,
                role: Role::Member,
            }],
            sessions: vec![SeedSession { token, user_id: id }],
            logins: vec![who.login()],
            api_keys: vec![who.api_key()],
            ..SeedFixture::default()
        };
        let r = t.admin_post("/internal/seed", &serde_json::to_value(&f).unwrap()).await;
        assert_eq!(r.status(), 204);
        who
    }
}

/// `{SLICE_PREFIX}/workspaces/{slug}/projects/{project}`.
pub fn project_path(at: &At) -> String {
    format!("{SLICE_PREFIX}/workspaces/{}/projects/{}", at.slug, at.project)
}

pub fn wi_path(at: &At) -> String {
    format!("{}/work-items", project_path(at))
}

pub async fn create(t: &Target, who: &User, at: &At, body: Value) -> reqwest::Response {
    t.http.post(t.url(&wi_path(at))).bearer_auth(&who.token).json(&body).send().await.unwrap()
}

pub async fn create_ok(t: &Target, who: &User, at: &At, name: &str) -> WorkItem {
    let r = create(t, who, at, json!({ "name": name })).await;
    assert_eq!(r.status(), 201, "create '{name}' failed: {:?}", r.text().await);
    r.json::<WorkItem>().await.unwrap()
}

/// `GET .../work-items/search?q=`: the work items of the project whose name or description contains `q`, any case.
pub fn search_path(at: &At) -> String {
    format!("{}/search", wi_path(at))
}

pub async fn search(t: &Target, who: &User, at: &At, q: &str) -> reqwest::Response {
    t.http.get(t.url(&search_path(at))).query(&[("q", q)]).bearer_auth(&who.token).send().await.unwrap()
}

/// `POST /internal/seed-work-items`: bulk-insert live work items into a project, numbered from its counter (what the
/// bench and the search parity test seed 100k rows with). Names and descriptions are the caller's.
pub async fn seed_work_items(
    t: &Target,
    f: &Fixture,
    at: &At,
    created_by: UserId,
    items: &[(String, Option<String>)],
) -> SeedWorkItemsReport {
    let workspace_id = if at.project == f.a.project { f.ws_a } else { f.ws_b };
    let req = SeedWorkItemsRequest {
        workspace_id,
        project_id: at.project,
        created_by,
        items: items
            .iter()
            .map(|(name, description)| SeedWorkItemSpec { name: name.clone(), description: description.clone() })
            .collect(),
    };
    let r = t.admin_post("/internal/seed-work-items", &serde_json::to_value(&req).unwrap()).await;
    assert_eq!(r.status(), 200, "seed-work-items failed: {:?}", r.text().await);
    r.json().await.unwrap()
}

pub async fn list(t: &Target, who: &User, at: &At, scope: Option<&str>) -> reqwest::Response {
    let mut u = t.url(&wi_path(at));
    if let Some(s) = scope {
        u.push_str(&format!("?scope={s}"));
    }
    t.http.get(u).bearer_auth(&who.token).send().await.unwrap()
}

pub async fn activity(t: &Target, who: &User, at: &At, item: WorkItemId) -> Vec<Activity> {
    let r =
        t.http.get(t.url(&format!("{}/{}/activity", wi_path(at), item))).bearer_auth(&who.token).send().await.unwrap();
    assert_eq!(r.status(), 200, "activity");
    r.json::<Vec<Activity>>().await.unwrap()
}

// --- `/api/v1`: Plane's wire shape, authenticated by API key --------------------------------------------------------------------

/// Where Plane's v1 API is served (ADR-0031), and the header it authenticates by.
pub const V1_PREFIX: &str = "/api/v1";
pub const API_KEY_HEADER: &str = "x-api-key";

/// The work items of a project through `/api/v1`: `{V1_PREFIX}/workspaces/{slug}/projects/{project}/issues/`.
pub fn v1_items(at: &At) -> String {
    format!("{V1_PREFIX}/workspaces/{}/projects/{}/issues/", at.slug, at.project)
}

/// One work item: `.../issues/{id}/`.
pub fn v1_item(at: &At, id: &str) -> String {
    format!("{}{id}/", v1_items(at))
}

/// A work item as `/api/v1` answers it: Plane's 29-key body, read for the few keys a contract test looks at. The rest of the body
/// is the oracle's (`tools/oracle-v1`), not this suite's.
#[derive(Debug, Clone)]
pub struct Issue(pub Value);

impl Issue {
    pub fn id(&self) -> String {
        self.0["id"].as_str().expect("a work item has an id").to_string()
    }

    pub fn name(&self) -> &str {
        self.0["name"].as_str().expect("a work item has a name")
    }

    pub fn sequence_id(&self) -> i64 {
        self.0["sequence_id"].as_i64().expect("a work item has a sequence_id")
    }

    /// Plane writes it as a float (`65535.0`).
    pub fn sort_order(&self) -> f64 {
        self.0["sort_order"].as_f64().expect("a work item has a sort_order")
    }

    pub fn workspace(&self) -> Option<WorkspaceId> {
        self.0["workspace"].as_str().and_then(|s| uuid::Uuid::parse_str(s).ok()).map(WorkspaceId)
    }

    pub fn created_by(&self) -> Option<UserId> {
        self.0["created_by"].as_str().and_then(|s| uuid::Uuid::parse_str(s).ok()).map(UserId)
    }
}

/// `POST .../issues/` with `who`'s API key.
pub async fn v1_create(t: &Target, who: &User, at: &At, body: Value) -> reqwest::Response {
    t.http.post(t.url(&v1_items(at))).header(API_KEY_HEADER, &who.key).json(&body).send().await.unwrap()
}

/// A create that must answer 201 (Plane's own answer for a work item, the corpus's `create`).
pub async fn v1_create_ok(t: &Target, who: &User, at: &At, name: &str) -> Issue {
    let r = v1_create(t, who, at, json!({ "name": name })).await;
    assert_eq!(r.status(), 201, "create '{name}' failed: {:?}", r.text().await);
    Issue(r.json().await.unwrap())
}

/// `GET .../issues/` with `who`'s API key, a hundred to a page (more than any test of this suite makes).
pub async fn v1_list(t: &Target, who: &User, at: &At) -> reqwest::Response {
    t.http
        .get(t.url(&v1_items(at)))
        .query(&[("per_page", "100")])
        .header(API_KEY_HEADER, &who.key)
        .send()
        .await
        .unwrap()
}

/// The work items of a list that must answer 200, newest first (the order of the pin's list).
pub async fn v1_list_ok(t: &Target, who: &User, at: &At) -> Vec<Issue> {
    let r = v1_list(t, who, at).await;
    assert_eq!(r.status(), 200, "list failed");
    let body: Value = r.json().await.unwrap();
    let results = body["results"].as_array().expect("a list has `results`");
    assert_eq!(body["total_count"].as_u64(), Some(results.len() as u64), "one page holds the whole list: {body}");
    results.iter().cloned().map(Issue).collect()
}

/// `DELETE .../issues/{id}/` with `who`'s API key.
pub async fn v1_delete(t: &Target, who: &User, at: &At, id: &str) -> reqwest::Response {
    t.http.delete(t.url(&v1_item(at, id))).header(API_KEY_HEADER, &who.key).send().await.unwrap()
}

/// Every work item of a project, a thousand to a page (the most Plane allows) by its cursor (`limit:page:0`), in the order of the
/// pin's list. For a project with more rows than one page: the engine tests make ten thousand.
pub async fn v1_list_all(t: &Target, who: &User, at: &At) -> Vec<Issue> {
    let mut all = Vec::new();
    let mut cursor = "1000:0:0".to_string();
    loop {
        let r = t
            .http
            .get(t.url(&v1_items(at)))
            .query(&[("per_page", "1000"), ("cursor", cursor.as_str())])
            .header(API_KEY_HEADER, &who.key)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "list page {cursor}");
        let body: Value = r.json().await.unwrap();
        all.extend(body["results"].as_array().expect("a list has `results`").iter().cloned().map(Issue));
        if body["next_page_results"].as_bool() != Some(true) {
            return all;
        }
        cursor = body["next_cursor"].as_str().expect("a page has a next_cursor").to_string();
    }
}

/// `GET {V1_PREFIX}/workspaces/{slug}/issues/search/?search=q`: the work items of the caller's own projects in the workspace that a
/// name, a project identifier or a number says, newest first, at most `limit` of them (the pin's default is ten).
pub async fn v1_search(t: &Target, who: &User, slug: &str, q: &str, limit: u32) -> reqwest::Response {
    t.http
        .get(t.url(&format!("{V1_PREFIX}/workspaces/{slug}/issues/search/")))
        .query(&[("search", q), ("limit", &limit.to_string())])
        .header(API_KEY_HEADER, &who.key)
        .send()
        .await
        .unwrap()
}

/// The hits of a search that must answer 200: Plane's `issues` (`name`, `id`, `sequence_id`, `project__identifier`, `project_id`,
/// `workspace__slug`).
pub async fn v1_search_hits(t: &Target, who: &User, slug: &str, q: &str, limit: u32) -> Vec<Value> {
    let r = v1_search(t, who, slug, q, limit).await;
    let status = r.status();
    assert_eq!(status, 200, "search for {q:?}: HTTP {status}: {:?}", r.text().await);
    let body: Value = r.json().await.unwrap();
    body["issues"].as_array().unwrap_or_else(|| panic!("a search answers `issues`: {body}")).clone()
}

/// Poll until `pred` holds (queue consumers are asynchronous by design). Fails loudly on timeout.
pub async fn eventually<F, Fut, T>(what: &str, timeout: Duration, mut f: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let start = std::time::Instant::now();
    loop {
        if let Some(v) = f().await {
            return v;
        }
        if start.elapsed() > timeout {
            panic!("timed out after {:?} waiting for: {what}", timeout);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
