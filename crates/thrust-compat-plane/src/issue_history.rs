//! A work item's history (X02), and the other calls its page makes when the app opens it (the sub work items, the relations).
//!
//! The history is what the pin's `IssueActivityEndpoint` answers: the changes made to the work item, oldest first, each with the
//! work item as it is now, its project, its workspace and who made the change. The rows are written by the activity consumer from
//! the events a creation and an update carry (`thrust_core::issues`), so a change is seen in the history a moment after it is made,
//! as on the pin, where a task on its worker writes them. Comments are not built (nothing makes one): the list of them is empty,
//! and the history without a kind asked for is the changes alone.
//!
//! Every member of the project may read any work item's history, a guest too and whether or not the project shows guests
//! everything; the pin's guard is a permission class, so a refusal is DRF's own 403, not the decorator's.
//!
//! The sub work items and the relations (D-021, a stub): a work item has none, so the answers are the pin's empty ones. A work
//! item that is not there is `[]`-like for the sub work items (the pin does not look) and a 404 for the relations (it does).

use std::collections::HashMap;

use axum::extract::{Path, Query};
use axum::{Extension, Json};
use serde_json::{json, Value};
use thrust_contracts::*;
use thrust_core::{App, History, IssueAccess, IssueError};
use thrust_ports::*;

use crate::issues::{date_or_null, datetime, issue_id, issue_not_found};
use crate::projects::project_id;
use crate::users::{non_empty, ts, Caller};
use crate::workspace_load::refused_by_class;
use crate::workspaces::id_or_null;
use crate::PlaneError;

/// A refusal of a view guarded by a permission class: DRF's own 403 whatever the reason, and the core's error as it is when it is
/// not a refusal.
fn by_class(e: IssueError) -> PlaneError {
    match e {
        IssueError::Core(e) => e.into(),
        IssueError::Forbidden
        | IssueError::NotFound
        | IssueError::NotAllowedToView
        | IssueError::Fields(_)
        | IssueError::StartAfterTarget => refused_by_class(),
    }
}

/// The caller's way into the project's work items for a view guarded by a permission class: any member of the project.
async fn member(app: &App, who: &Principal, slug: String, project: ProjectId) -> Result<IssueAccess, PlaneError> {
    thrust_core::issue_reader(app, who, &WorkspaceSlug(slug), project).await.map_err(by_class)
}

/// The person who made a change, as the app shows one (`UserLiteSerializer`).
fn actor_json(user: UserId, p: &UserProfile) -> Value {
    json!({
        "id": user.0,
        "first_name": p.first_name,
        "last_name": p.last_name,
        "avatar": p.avatar,
        "avatar_url": non_empty(&p.avatar),
        "is_bot": false,
        "display_name": p.display_name,
    })
}

/// One row of the history: the pin's `IssueActivitySerializer`, every column and the four details.
fn activity_json(
    row: &WorkItemActivity,
    item: &WorkItemRow,
    h: &History,
    project: &ProjectRow,
    workspace: &Workspace,
) -> Value {
    let (i, a) = (&item.item, &item.attrs);
    json!({
        "id": row.id.0,
        "actor_detail": row.actor_id.and_then(|u| h.people.get(&u).map(|p| actor_json(u, p))),
        "issue_detail": {
            "id": i.id.0,
            "name": i.name,
            "description_json": {},
            "description_html": a.description_html,
            "priority": a.priority.as_str(),
            "start_date": date_or_null(a.start_date),
            "target_date": date_or_null(a.target_date),
            "sequence_id": i.sequence_id,
            "sort_order": i.sort_order as f64,
            "is_draft": i.is_draft,
        },
        "project_detail": {
            "id": project.id.0,
            "identifier": project.identifier,
            "name": project.name,
            "cover_image": project.cover_image,
            "cover_image_url": project.cover_image,
            "logo_props": project.logo_props,
            "description": project.description,
        },
        "workspace_detail": {"name": workspace.name, "slug": workspace.slug.0, "id": workspace.id.0, "logo_url": null},
        // the work item's intake, which a work item made here never has
        "source_data": null,
        "created_at": ts(row.created_at),
        "updated_at": ts(row.created_at),
        "deleted_at": null,
        "verb": row.verb,
        "field": row.field,
        "old_value": row.old_value,
        "new_value": row.new_value,
        "comment": row.comment,
        "attachments": [],
        "old_identifier": row.old_identifier,
        "new_identifier": row.new_identifier,
        "epoch": row.epoch as f64,
        "created_by": null,
        "updated_by": null,
        "project": row.project_id.0,
        "workspace": row.workspace_id.0,
        "issue": row.work_item_id.0,
        "issue_comment": null,
        "actor": id_or_null(row.actor_id),
    })
}

/// X02. A work item's history, oldest first. `activity_type=issue-comment` asks for the comments, of which there are none; every
/// other value (the app sends `issue-property`) and none at all is the changes. `created_at__gt` keeps those made after a time; a
/// time that is not one is 400, as for the list's `updated_at__gt`. A work item that is not there has no history, not a 404.
pub(crate) async fn history(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project, issue)): Path<(String, String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, PlaneError> {
    let project = project_id(&project)?;
    let issue = issue_id(&issue)?;
    let access = member(&app, &who, slug, project).await?;
    let after = match q.get("created_at__gt") {
        Some(raw) => {
            Some(datetime(raw).ok_or_else(|| PlaneError::BadRequest(json!({"error": "Please provide valid detail"})))?)
        }
        None => None,
    };
    if q.get("activity_type").map(String::as_str) == Some("issue-comment") {
        return Ok(Json(json!([])));
    }
    let h = thrust_core::issue_history(&app, &access, issue, after).await.map_err(by_class)?;
    let (Some(item), Some(project), Some(workspace)) = (&h.item, &h.project, &h.workspace) else {
        return Ok(Json(json!([])));
    };
    Ok(Json(Value::Array(h.rows.iter().map(|r| activity_json(r, item, &h, project, workspace)).collect())))
}

/// The work item's sub work items, which the app asks for when it opens one. A stub: there are none (a work item has no parent),
/// and the pin does not look for the work item first.
pub(crate) async fn sub_issues(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project, issue)): Path<(String, String, String)>,
) -> Result<Json<Value>, PlaneError> {
    let project = project_id(&project)?;
    issue_id(&issue)?;
    member(&app, &who, slug, project).await?;
    Ok(Json(json!({"sub_issues": [], "state_distribution": {}})))
}

/// The work item's relations to others, which the app asks for when it opens one. A stub: there are none. The work item must be
/// there (the pin looks).
pub(crate) async fn relations(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project, issue)): Path<(String, String, String)>,
) -> Result<Json<Value>, PlaneError> {
    let project = project_id(&project)?;
    let issue = issue_id(&issue)?;
    let access = member(&app, &who, slug, project).await?;
    if !thrust_core::issue_there(&app, &access, issue).await.map_err(by_class)? {
        return Err(issue_not_found());
    }
    Ok(Json(json!({
        "blocking": [],
        "blocked_by": [],
        "duplicate": [],
        "relates_to": [],
        "start_after": [],
        "start_before": [],
        "finish_after": [],
        "finish_before": [],
    })))
}
