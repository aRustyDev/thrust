//! Every actor × slug × project × route, once, against the access table in the parent module (`standing`, `gate`)
//! and the slug rule below. The routes are the ones no Plane route replaces: the activity feed and the three attachment
//! routes. (The work-item routes' own table, who may make, change, delete and read, is `/api/v1`'s: `v1_model::matrix`.)

use super::*;

#[derive(Debug, Clone, Copy)]
enum Route {
    Activity,
    PutAttachment,
    GetAttachment,
    DeleteAttachment,
}

const ALL_ROUTES: [Route; 4] = [Route::Activity, Route::PutAttachment, Route::GetAttachment, Route::DeleteAttachment];

fn min_role(r: Route) -> Role {
    match r {
        Route::PutAttachment | Route::DeleteAttachment => Role::Member,
        Route::Activity | Route::GetAttachment => Role::Guest,
    }
}

fn success_status(r: Route) -> u16 {
    match r {
        Route::Activity | Route::GetAttachment => 200,
        Route::PutAttachment | Route::DeleteAttachment => 204,
    }
}

/// The status a request that names `slug` gets when it needs `min`, or `None` when it passes. Authentication comes
/// first. Then the slug: one that is not the project's own workspace's names another workspace, or none, and that is
/// 404 whoever asks (the project is not in the workspace it names). Then the access table (`gate`).
fn rejection(actor: Actor, slug: Slug, proj: Proj, min: Role) -> Option<u16> {
    match (standing(actor, proj), slug) {
        (Standing::Unauthenticated, _) => Some(401),
        (_, Slug::Other | Slug::Unknown) => Some(404),
        (_, Slug::Own) => gate(actor, proj, min).map(|(status, _)| status),
    }
}

/// Every combination is checked (7 × 3 × 3 × 4 = 252) and every disagreement is reported, not only the first. The
/// attachment routes work on one live item per project (made through `/api/v1` by that project's admin) under a name
/// of their own; for a get or a delete the admin stores the object first. Afterwards both projects must hold exactly
/// the objects the table predicts (no rejected request had an effect).
#[tokio::test]
async fn every_actor_project_and_route_gets_the_status_of_the_access_table() {
    let t = Target::from_env();
    let w = World::seed(&t).await;
    let admin = |s: usize| if s == 0 { &w.f.admin_a } else { &w.f.admin_b };
    // per project: (attachment name, whether it must exist at the end) on that project's host item
    let mut want_blobs: [Vec<(String, bool)>; 2] = [Vec::new(), Vec::new()];
    let host = [
        v1_create_ok(&t, admin(0), w.at_slot(0), "attachment host").await.work_item_id(),
        v1_create_ok(&t, admin(1), w.at_slot(1), "attachment host").await.work_item_id(),
    ];
    let mut mismatches = Vec::new();
    let mut checked = 0;
    for actor in ALL_ACTORS {
        for slug in ALL_SLUGS {
            for proj in ALL_PROJECTS {
                for route in ALL_ROUTES {
                    let label = format!("{actor:?} {route:?} on {proj:?} via {slug:?}");
                    let allowed = rejection(actor, slug, proj, min_role(route)).is_none();
                    let req = match route {
                        Route::PutAttachment | Route::GetAttachment | Route::DeleteAttachment => {
                            let name = format!("m{checked}.txt");
                            let item = slot(proj).map_or(unknown_item(checked), |s| host[s]);
                            let at = |actor, method, slug| Req {
                                item: Some(item),
                                attachment: Some(name.clone()),
                                slug,
                                ..Req::bare(actor, method, proj)
                            };
                            if let Some(s) = slot(proj) {
                                if !matches!(route, Route::PutAttachment) {
                                    // the setup is always the admin, through the project's own workspace
                                    let put = Req {
                                        upload: Some(Stored { size: Size::Small(16), seed: 1, content_type: None }),
                                        ..at(admin_of(s), Method::PUT, Slug::Own)
                                    };
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
                                Route::PutAttachment => Req {
                                    upload: Some(Stored { size: Size::Small(8), seed: 2, content_type: None }),
                                    ..at(actor, Method::PUT, slug)
                                },
                                Route::GetAttachment => at(actor, Method::GET, slug),
                                _ => at(actor, Method::DELETE, slug),
                            })
                        }
                        // not a work-item path, so not a `Req`
                        Route::Activity => None,
                    };
                    let status = match req {
                        Some(req) => w.send(&req).await.status,
                        None => {
                            let url = t.url(&format!("{}/activity", project_path(&w.at(proj, slug))));
                            w.as_actor(t.http.get(url), actor).send().await.unwrap().status().as_u16()
                        }
                    };
                    let want = rejection(actor, slug, proj, min_role(route)).unwrap_or(success_status(route));
                    if status != want {
                        mismatches.push(format!("{label}: got {status}, the table says {want}"));
                    }
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(
        checked,
        ALL_ACTORS.len() * ALL_SLUGS.len() * ALL_PROJECTS.len() * ALL_ROUTES.len(),
        "the matrix did not run"
    );
    for (s, blobs) in want_blobs.iter().enumerate() {
        for (name, exists) in blobs {
            let proj = [Proj::A, Proj::B][s];
            let get = Req {
                item: Some(host[s]),
                attachment: Some(name.clone()),
                ..Req::bare(admin_of(s), Method::GET, proj)
            };
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
