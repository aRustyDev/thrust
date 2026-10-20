//! Every actor × project × route, once, against the access table in the parent module (`standing`, `gate`).

use super::*;

#[derive(Debug, Clone, Copy)]
enum Route {
    Create,
    List,
    ListWithDeleted,
    Delete,
    Activity,
}

const ALL_ROUTES: [Route; 5] = [Route::Create, Route::List, Route::ListWithDeleted, Route::Delete, Route::Activity];

fn min_role(r: Route) -> Role {
    match r {
        Route::Create | Route::Delete => Role::Member,
        Route::List | Route::Activity => Role::Guest,
        Route::ListWithDeleted => Role::Admin,
    }
}

fn success_status(r: Route) -> u16 {
    match r {
        Route::Create => 201,
        Route::List | Route::ListWithDeleted | Route::Activity => 200,
        Route::Delete => 204,
    }
}

/// Every combination is checked (7 × 3 × 5 = 105) and every disagreement is reported, not only the first. A delete
/// targets a fresh live item created by that project's admin, so an allowed delete and a rejected one are both
/// observable; afterwards both projects must hold exactly the rows the table predicts (no rejected request had an
/// effect).
#[tokio::test]
async fn every_actor_project_and_route_gets_the_status_of_the_access_table() {
    let t = Target::from_env();
    let w = World::seed(&t).await;
    let admin = |s: usize| if s == 0 { &w.f.admin_a } else { &w.f.admin_b };
    // per project: (name, deleted) of every row the table says must exist at the end
    let mut want_rows: [Vec<(String, bool)>; 2] = [Vec::new(), Vec::new()];
    let mut mismatches = Vec::new();
    let mut checked = 0;
    for actor in ALL_ACTORS {
        for proj in ALL_PROJECTS {
            for route in ALL_ROUTES {
                let label = format!("{actor:?} {route:?} on {proj:?}");
                let allowed = gate(actor, proj, min_role(route)).is_none();
                let mut victim = None;
                let req = match route {
                    Route::Create => Some(Req {
                        actor,
                        method: Method::POST,
                        proj,
                        item: None,
                        query: None,
                        body: Some(json!({ "name": label })),
                    }),
                    Route::List | Route::ListWithDeleted => Some(Req {
                        actor,
                        method: Method::GET,
                        proj,
                        item: None,
                        query: matches!(route, Route::ListWithDeleted).then_some("with_deleted"),
                        body: None,
                    }),
                    Route::Delete => {
                        let item = match slot(proj) {
                            Some(s) => {
                                let v = create_ok(&t, admin(s), w.project(proj), &format!("victim of {label}")).await;
                                victim = Some(s);
                                v.id
                            }
                            None => unknown_item(checked),
                        };
                        Some(Req { actor, method: Method::DELETE, proj, item: Some(item), query: None, body: None })
                    }
                    // not a work-item path, so not a `Req`
                    Route::Activity => None,
                };
                let status = match req {
                    Some(req) => w.send(&req).await.0,
                    None => {
                        let url = t.url(&format!("/api/v1/projects/{}/activity", w.project(proj)));
                        w.as_actor(t.http.get(url), actor).send().await.unwrap().status().as_u16()
                    }
                };
                let want = gate(actor, proj, min_role(route)).map_or(success_status(route), |(s, _)| s);
                if status != want {
                    mismatches.push(format!("{label}: got {status}, the table says {want}"));
                }
                if let Some(s) = victim {
                    want_rows[s].push((format!("victim of {label}"), allowed));
                }
                if matches!(route, Route::Create) && allowed {
                    want_rows[slot(proj).unwrap()].push((label.clone(), false));
                }
                checked += 1;
            }
        }
    }
    assert_eq!(checked, ALL_ACTORS.len() * ALL_PROJECTS.len() * ALL_ROUTES.len(), "the matrix did not run");
    for (s, want) in want_rows.iter_mut().enumerate() {
        let items: Vec<WorkItem> =
            list(&t, admin(s), w.scope_of(s).1, Some("with_deleted")).await.json().await.unwrap();
        let mut got: Vec<(String, bool)> = items.iter().map(|i| (i.name.clone(), i.deleted_at.is_some())).collect();
        got.sort();
        want.sort();
        if got != *want {
            mismatches.push(format!("project {s}: rows after the matrix\n  got  {got:?}\n  want {want:?}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} disagreement(s) with the access table over {checked} combinations:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
