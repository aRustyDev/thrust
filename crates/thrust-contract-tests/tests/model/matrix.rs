//! Every actor × project × route, once, against the access table in the parent module (`standing`, `gate`).

use super::*;

#[derive(Debug, Clone, Copy)]
enum Route {
    Create,
    List,
    ListWithDeleted,
    Delete,
    Activity,
    PutAttachment,
    GetAttachment,
    DeleteAttachment,
}

const ALL_ROUTES: [Route; 8] = [
    Route::Create,
    Route::List,
    Route::ListWithDeleted,
    Route::Delete,
    Route::Activity,
    Route::PutAttachment,
    Route::GetAttachment,
    Route::DeleteAttachment,
];

fn min_role(r: Route) -> Role {
    match r {
        Route::Create | Route::Delete | Route::PutAttachment | Route::DeleteAttachment => Role::Member,
        Route::List | Route::Activity | Route::GetAttachment => Role::Guest,
        Route::ListWithDeleted => Role::Admin,
    }
}

fn success_status(r: Route) -> u16 {
    match r {
        Route::Create => 201,
        Route::List | Route::ListWithDeleted | Route::Activity | Route::GetAttachment => 200,
        Route::Delete | Route::PutAttachment | Route::DeleteAttachment => 204,
    }
}

/// Every combination is checked (7 × 3 × 8 = 168) and every disagreement is reported, not only the first. A delete
/// targets a fresh live item created by that project's admin, so an allowed delete and a rejected one are both
/// observable. The attachment routes work on one live item per project under a name of their own; for a get or a
/// delete the admin stores the object first. Afterwards both projects must hold exactly the rows and the objects the
/// table predicts (no rejected request had an effect).
#[tokio::test]
async fn every_actor_project_and_route_gets_the_status_of_the_access_table() {
    let t = Target::from_env();
    let w = World::seed(&t).await;
    let admin = |s: usize| if s == 0 { &w.f.admin_a } else { &w.f.admin_b };
    // per project: (name, deleted) of every row the table says must exist at the end
    let mut want_rows: [Vec<(String, bool)>; 2] = [Vec::new(), Vec::new()];
    // per project: (attachment name, whether it must exist at the end) on that project's host item
    let mut want_blobs: [Vec<(String, bool)>; 2] = [Vec::new(), Vec::new()];
    let host = [
        create_ok(&t, admin(0), w.scope_of(0).1, "attachment host").await.id,
        create_ok(&t, admin(1), w.scope_of(1).1, "attachment host").await.id,
    ];
    for rows in &mut want_rows {
        rows.push(("attachment host".to_string(), false));
    }
    let mut mismatches = Vec::new();
    let mut checked = 0;
    for actor in ALL_ACTORS {
        for proj in ALL_PROJECTS {
            for route in ALL_ROUTES {
                let label = format!("{actor:?} {route:?} on {proj:?}");
                let allowed = gate(actor, proj, min_role(route)).is_none();
                let mut victim = None;
                let req = match route {
                    Route::Create => Some(Req { body: Some(json!({ "name": label })), ..Req::bare(actor, Method::POST, proj) }),
                    Route::List | Route::ListWithDeleted => Some(Req {
                        query: matches!(route, Route::ListWithDeleted).then_some("with_deleted"),
                        ..Req::bare(actor, Method::GET, proj)
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
                        Some(Req { item: Some(item), ..Req::bare(actor, Method::DELETE, proj) })
                    }
                    Route::PutAttachment | Route::GetAttachment | Route::DeleteAttachment => {
                        let name = format!("m{checked}.txt");
                        let item = slot(proj).map_or(unknown_item(checked), |s| host[s]);
                        let at = |actor, method| Req { item: Some(item), attachment: Some(name.clone()), ..Req::bare(actor, method, proj) };
                        if let Some(s) = slot(proj) {
                            if !matches!(route, Route::PutAttachment) {
                                let put = Req { upload: Some(Stored { size: Size::Small(16), seed: 1, content_type: None }), ..at(admin_of(s), Method::PUT) };
                                assert_eq!(w.send(&put).await.status, 204, "{label}: the admin's setup put");
                            }
                            let exists = match route {
                                Route::PutAttachment => allowed,
                                Route::DeleteAttachment => !allowed,
                                _ => true,
                            };
                            want_blobs[s].push((name.clone(), exists));
                        }
                        Some(match route {
                            Route::PutAttachment => Req { upload: Some(Stored { size: Size::Small(8), seed: 2, content_type: None }), ..at(actor, Method::PUT) },
                            Route::GetAttachment => at(actor, Method::GET),
                            _ => at(actor, Method::DELETE),
                        })
                    }
                    // not a work-item path, so not a `Req`
                    Route::Activity => None,
                };
                let status = match req {
                    Some(req) => w.send(&req).await.status,
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
    for (s, blobs) in want_blobs.iter().enumerate() {
        for (name, exists) in blobs {
            let proj = [Proj::A, Proj::B][s];
            let get = Req { item: Some(host[s]), attachment: Some(name.clone()), ..Req::bare(admin_of(s), Method::GET, proj) };
            let status = w.send(&get).await.status;
            let want = if *exists { 200 } else { 404 };
            if status != want {
                mismatches.push(format!("project {s}: attachment {name} after the matrix: got {status}, want {want}"));
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} disagreement(s) with the access table over {checked} combinations:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
