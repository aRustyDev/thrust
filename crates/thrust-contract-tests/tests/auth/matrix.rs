//! Every credential kind x credential state x method x CSRF presence, once, against the status table below (issue #38,
//! ADR-0052, ADR-0033 round 27). 4 kinds x their states x 3 methods x 3 token positions = 126 requests, every
//! disagreement reported, and afterwards the project holds exactly the rows the table predicts: no refused request had
//! an effect.
//!
//! The table, written out:
//!
//! * no credential: 401;
//! * a bearer token: 401 unless it is a live session; the CSRF header is never looked at (it is not an ambient credential);
//! * the session cookie: 401 unless it is a live session (checked first, so a dead cookie is 401 whatever the header
//!   says); a live one is accepted for a safe method, and for a mutation only with the CSRF header equal to the CSRF
//!   cookie, else 403;
//! * `X-Api-Key`: 403 unless it is live (an unknown, revoked, expired or inactive-user key are all 403: ADR-0033), the
//!   CSRF header is never looked at.
//!
//! "Live" for a session is: issued by sign-in, not signed out, and its user active. Sessions cannot be made to expire
//! over HTTP in a test; expiry is the core's (`authenticate_resolves_a_live_session_and_nothing_else`).

use reqwest::Method;
use serde_json::json;
use thrust_contract_tests::*;
use thrust_contracts::*;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Nothing,
    Bearer,
    Cookie,
    ApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum St {
    Live,
    Revoked,
    Expired,
    OwnerInactive,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum M {
    List,
    Create,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    /// The CSRF header carries the token the CSRF cookie holds.
    Matching,
    Absent,
    Wrong,
}

fn mutating(m: M) -> bool {
    !matches!(m, M::List)
}

fn success(m: M) -> u16 {
    match m {
        M::List => 200,
        M::Create => 201,
        M::Delete => 204,
    }
}

/// The states a kind can be in over HTTP (a session cannot be expired in a test).
fn states(kind: Kind) -> &'static [St] {
    match kind {
        Kind::Nothing => &[St::Live],
        Kind::Bearer | Kind::Cookie => &[St::Live, St::Revoked, St::OwnerInactive, St::Unknown],
        Kind::ApiKey => &[St::Live, St::Revoked, St::Expired, St::OwnerInactive, St::Unknown],
    }
}

/// The spec.
fn expected(kind: Kind, st: St, m: M, tok: Tok) -> u16 {
    let live = st == St::Live;
    match kind {
        Kind::Nothing => 401,
        Kind::Bearer if live => success(m),
        Kind::Bearer => 401,
        Kind::Cookie if live && (!mutating(m) || tok == Tok::Matching) => success(m),
        Kind::Cookie if live => 403,
        Kind::Cookie => 401,
        Kind::ApiKey if live => success(m),
        Kind::ApiKey => 403,
    }
}

/// One credential of every state, for the acting user (a member of project A) and for a second user who is deactivated
/// after their credentials were made.
struct Creds {
    live: Session,
    revoked: Session,
    inactive: Session,
    unknown: Session,
    live_key: String,
    revoked_key: String,
    expired_key: String,
    inactive_key: String,
    unknown_key: String,
}

impl Creds {
    fn session(&self, st: St) -> &Session {
        match st {
            St::Live => &self.live,
            St::Revoked => &self.revoked,
            St::OwnerInactive => &self.inactive,
            St::Unknown | St::Expired => &self.unknown,
        }
    }

    fn key(&self, st: St) -> &str {
        match st {
            St::Live => &self.live_key,
            St::Revoked => &self.revoked_key,
            St::Expired => &self.expired_key,
            St::OwnerInactive => &self.inactive_key,
            St::Unknown => &self.unknown_key,
        }
    }
}

async fn make_creds(t: &Target, f: &Fixture, ghost: &User) -> Creds {
    let live = t.sign_in(&f.member_a).await;
    let revoked = t.sign_in(&f.member_a).await;
    t.revoke_session(&revoked.token).await;
    let inactive = t.sign_in(ghost).await;
    let unknown = Session {
        user_id: f.member_a.id,
        token: format!("unknown-{}", uuid::Uuid::new_v4().simple()),
        csrf: "0123456789abcdef0123456789abcdef".into(),
    };
    let live_key = create_key_ok(t, &f.member_a, json!({})).await.token;
    let revoked = (revoked, create_key_ok(t, &f.member_a, json!({})).await);
    let r = t.http.delete(format!("{}/{}", keys_url(t), revoked.1.id)).bearer_auth(&f.member_a.token).send().await.unwrap();
    assert_eq!(r.status(), 204, "revoke the key that is to be revoked");
    let inactive_key = create_key_ok(t, ghost, json!({})).await.token;
    let expired_key = format!("thrk_expired-{}", uuid::Uuid::new_v4().simple());
    seed_keys(
        t,
        vec![SeedApiKey {
            token: expired_key.clone(),
            user_id: f.member_a.id,
            label: "expired".into(),
            expires_at: Some(thrust_contracts::chrono::DateTime::from_timestamp(1_000_000_000, 0).unwrap()),
            revoked: false,
        }],
    )
    .await;
    // the ghost's credentials were made while it was active; now it is not
    t.set_user_active(ghost.id, false).await;
    Creds {
        live,
        revoked: revoked.0,
        inactive,
        unknown,
        live_key,
        revoked_key: revoked.1.token,
        expired_key,
        inactive_key,
        unknown_key: format!("thrk_unknown-{}", uuid::Uuid::new_v4().simple()),
    }
}

#[tokio::test]
async fn every_credential_state_method_and_csrf_position_gets_the_status_of_the_table() {
    let t = t();
    let f = Fixture::seed(&t).await;
    let ghost = f.extra_member(&t).await;
    let creds = make_creds(&t, &f, &ghost).await;
    let shared_victim = create_ok(&t, &f.admin_a, &f.a, "shared victim").await.id;

    let mut mismatches = Vec::new();
    let (mut checked, mut want_created, mut want_deleted) = (0usize, 0usize, 0usize);
    let mut victims_deleted = Vec::new();
    for kind in [Kind::Nothing, Kind::Bearer, Kind::Cookie, Kind::ApiKey] {
        for &st in states(kind) {
            for m in [M::List, M::Create, M::Delete] {
                for tok in [Tok::Matching, Tok::Absent, Tok::Wrong] {
                    let want = expected(kind, st, m, tok);
                    let label = format!("{kind:?} {st:?} {m:?} csrf={tok:?}");
                    // a delete that is expected to succeed gets a victim of its own; every refused one aims at the
                    // shared victim, which must still be live at the end (the create limit is 30 a window per user)
                    let victim = match (m, want) {
                        (M::Delete, 204) => {
                            let v = create_ok(&t, &f.admin_a, &f.a, &format!("victim of {label}")).await.id;
                            victims_deleted.push(v);
                            v
                        }
                        _ => shared_victim,
                    };
                    let (method, url) = match m {
                        M::List => (Method::GET, t.url(&wi_path(&f.a))),
                        M::Create => (Method::POST, t.url(&wi_path(&f.a))),
                        M::Delete => (Method::DELETE, t.url(&format!("{}/{victim}", wi_path(&f.a)))),
                    };
                    let mut r = t.http.request(method, url);
                    if m == M::Create {
                        r = r.json(&json!({ "name": label }));
                    }
                    // the CSRF cookie that goes with the credential, if there is a cookie at all
                    let csrf_cookie = creds.session(st).csrf.clone();
                    r = match kind {
                        Kind::Nothing => r,
                        Kind::Bearer => r.bearer_auth(&creds.session(st).token),
                        Kind::Cookie => r.header("cookie", creds.session(st).cookie_header()),
                        Kind::ApiKey => r.header(API_KEY_HEADER, creds.key(st)),
                    };
                    r = match tok {
                        Tok::Matching => r.header(CSRF_HEADER, &csrf_cookie),
                        Tok::Absent => r,
                        Tok::Wrong => r.header(CSRF_HEADER, "ffffffffffffffffffffffffffffffff"),
                    };
                    let got = r.send().await.unwrap().status().as_u16();
                    if got != want {
                        mismatches.push(format!("{label}: got {got}, the table says {want}"));
                    }
                    if m == M::Create && want == 201 {
                        want_created += 1;
                    }
                    if m == M::Delete && want == 204 {
                        want_deleted += 1;
                    }
                    checked += 1;
                }
            }
        }
    }
    let combinations: usize = [Kind::Nothing, Kind::Bearer, Kind::Cookie, Kind::ApiKey].iter().map(|k| states(*k).len() * 9).sum();
    assert_eq!(checked, combinations, "the matrix did not run");
    assert_eq!(combinations, 126);

    // no refused request had an effect: the project holds the shared victim, exactly the rows the table let be created,
    // and exactly the victims it let be deleted (and they are the deleted ones)
    let rows: Vec<WorkItem> = list(&t, &f.admin_a, &f.a, Some("with_deleted")).await.json().await.unwrap();
    let created = rows.iter().filter(|r| r.created_by == f.member_a.id).count();
    let deleted: Vec<WorkItemId> = rows.iter().filter(|r| r.deleted_at.is_some()).map(|r| r.id).collect();
    let live_victim = rows.iter().find(|r| r.id == shared_victim).expect("the shared victim");
    if created != want_created {
        mismatches.push(format!("rows created by the acting user: {created}, the table lets {want_created}"));
    }
    if deleted.len() != want_deleted || !victims_deleted.iter().all(|v| deleted.contains(v)) {
        mismatches.push(format!("rows deleted: {}, the table lets {want_deleted}", deleted.len()));
    }
    if live_victim.deleted_at.is_some() {
        mismatches.push("a refused delete deleted the shared victim".into());
    }
    assert!(
        mismatches.is_empty(),
        "{} disagreement(s) with the table over {checked} combinations:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    // the table is not vacuous: it expects every one of these outcomes somewhere
    let outcomes: std::collections::BTreeSet<u16> = [Kind::Nothing, Kind::Bearer, Kind::Cookie, Kind::ApiKey]
        .iter()
        .flat_map(|k| states(*k).iter().map(move |s| (*k, *s)))
        .flat_map(|(k, s)| [M::List, M::Create, M::Delete].into_iter().map(move |m| (k, s, m)))
        .flat_map(|(k, s, m)| [Tok::Matching, Tok::Absent, Tok::Wrong].into_iter().map(move |t| expected(k, s, m, t)))
        .collect();
    assert_eq!(outcomes, [200, 201, 204, 401, 403].into_iter().collect());
}
