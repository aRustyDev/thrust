//! Work items as the compat plane serves them (issue #44, the work item stage): made, listed a page at a time, and opened.
//!
//! The rules are the pin's. A work item is made by an admin or a member of the project; a page of them and one of them are read
//! by any member, a guest too, except that a guest of a project that does not show guests everything sees only what they made. What
//! a request names must be there: its state is one of the project's, its assignees are users (and only those who are members of
//! the project, not guests, are kept), its labels are the project's. The wording and the order of a refusal are the compat layer's.
//!
//! A parent, an estimate, a cycle and a module are not built (deviation D-028): a work item has none of them, and a parent that a
//! request names is refused.

use thiserror::Error;
use thrust_contracts::chrono::NaiveDate;
use thrust_contracts::*;
use thrust_ports::*;

use crate::{
    authorize_with_role, authorize_workspace, event, relay_after_commit, App, CoreError, Permission, Principal,
};

/// Where the pin puts a new work item among the others in its state: 10000 after the greatest place any work item of the project
/// has in that state, or at 65535 when it is the first (`Issue.save`). The app shows and edits these numbers.
const FIRST_SORT_ORDER: i64 = 65_535;
const SORT_ORDER_STEP: i64 = 10_000;

/// What is wrong with one thing a request names, found by looking it up: DRF's "object does not exist" on a relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldProblem {
    /// The state is none of the project's.
    State,
    /// The parent is no work item of the project.
    Parent,
    /// The parent is one, but a work item has no parent here yet (deviation D-028).
    ParentNotSupported,
    /// The assignee at this place in the list is no user.
    Assignee(usize),
    /// The label at this place in the list is none of the project's.
    Label(usize),
}

/// Why a work item route refuses a caller or a request.
#[derive(Debug, Error)]
pub enum IssueError {
    /// Not a member of the project with a role the route allows (also: no such project, no such workspace). The pin's decorator
    /// answers all of them alike.
    #[error("forbidden")]
    Forbidden,
    /// No such work item (in the project, live).
    #[error("no such work item")]
    NotFound,
    /// A guest of a project that does not show guests everything asked for a work item they did not make.
    #[error("not allowed to view this work item")]
    NotAllowedToView,
    /// What the request names that is not there, in the order it names them.
    #[error("fields: {0:?}")]
    Fields(Vec<FieldProblem>),
    /// The start date is after the target date.
    #[error("start date cannot exceed target date")]
    StartAfterTarget,
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl From<DbError> for IssueError {
    fn from(e: DbError) -> Self {
        IssueError::Core(e.into())
    }
}

/// A caller the project does not let in is `Forbidden`, whatever the reason; anything else is the core's.
fn denied(e: CoreError) -> IssueError {
    match e {
        CoreError::NotFound | CoreError::Forbidden => IssueError::Forbidden,
        other => IssueError::Core(other),
    }
}

/// The proof that the caller may use a project's work items: what [`issue_reader`] or [`issue_writer`] made after the project let
/// them in. The use-cases take it, so a request is checked (and a guard's refusal answered) once, before the body or the query is
/// read, as the pin's decorator does.
#[derive(Debug, Clone)]
pub struct IssueAccess {
    scope: Scope,
    role: Role,
    who: Principal,
    slug: WorkspaceSlug,
}

impl IssueAccess {
    /// The caller's role in the project.
    pub fn role(&self) -> Role {
        self.role
    }
}

/// May the caller read the project's work items: an admin, a member or a guest of the project (`Forbidden` for anyone else).
pub async fn issue_reader(app: &App, who: &Principal, slug: &WorkspaceSlug, project: ProjectId) -> Result<IssueAccess, IssueError> {
    let (scope, role) = authorize_with_role(app, who, slug, project, Permission::ReadWorkItems).await.map_err(denied)?;
    Ok(IssueAccess { scope, role, who: who.clone(), slug: slug.clone() })
}

/// May the caller make work items in the project: an admin or a member (`Forbidden` for a guest and for anyone else).
pub async fn issue_writer(app: &App, who: &Principal, slug: &WorkspaceSlug, project: ProjectId) -> Result<IssueAccess, IssueError> {
    let (scope, role) = authorize_with_role(app, who, slug, project, Permission::CreateWorkItem).await.map_err(denied)?;
    Ok(IssueAccess { scope, role, who: who.clone(), slug: slug.clone() })
}

/// What a request to make a work item names, read and checked as far as the request alone allows (the compat layer's DRF
/// fields); the core looks up the rest.
#[derive(Debug, Clone, Default)]
pub struct IssueDraft {
    /// 1 to 255 characters once trimmed.
    pub name: String,
    /// Already cleaned of what must not be stored; `None` is the pin's empty paragraph.
    pub description_html: Option<String>,
    /// `None` is `none`.
    pub priority: Option<Priority>,
    /// `None` is the project's default state.
    pub state_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub start_date: Option<NaiveDate>,
    pub target_date: Option<NaiveDate>,
    pub assignee_ids: Vec<UserId>,
    pub label_ids: Vec<Uuid>,
}

/// What a page of the project's work items is asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueListQuery {
    pub order: WorkItemOrder,
    pub limit: u32,
    pub page: u32,
    pub updated_after: Option<Timestamp>,
}

/// Whether a guest of the project sees work items they did not make: the project's `guest_view_all_features`.
async fn guest_sees_all(app: &App, access: &IssueAccess) -> Result<bool, CoreError> {
    let workspace = authorize_workspace(app, &access.who, &access.slug).await?;
    Ok(app.ports.db.project_detail(&workspace, access.scope.project_id()).await?.is_some_and(|d| d.listing.project.guest_view_all_features))
}

/// Make a work item (the caller got [`issue_writer`]'s access). What the draft names is
/// looked up and what is missing is `Fields`; the dates must be in order; the item is numbered from the project's counter,
/// takes the project's default state when none is named (and `completed_at` when that state is of the `completed` group), and
/// is assigned to the project's default assignee when it has none left once those who may not be assigned are dropped. The item, its
/// attributes and its `work_item.created` event are one transaction.
pub async fn create_issue(app: &App, access: &IssueAccess, draft: IssueDraft) -> Result<WorkItemRow, IssueError> {
    let scope = access.scope;
    let db = &app.ports.db;

    let states = db.project_states(&scope).await?;
    let mut problems = Vec::new();
    if draft.state_id.is_some_and(|id| !states.iter().any(|s| s.id == id)) {
        problems.push(FieldProblem::State);
    }
    if let Some(parent) = draft.parent_id {
        problems.push(if db.work_item_row(&scope, WorkItemId(parent)).await?.is_some() { FieldProblem::ParentNotSupported } else { FieldProblem::Parent });
    }
    for (i, user) in draft.assignee_ids.iter().enumerate() {
        if db.user_by_id(*user).await?.is_none() {
            problems.push(FieldProblem::Assignee(i));
        }
    }
    if !draft.label_ids.is_empty() {
        let labels = db.project_labels(&scope).await?;
        for (i, label) in draft.label_ids.iter().enumerate() {
            if !labels.iter().any(|l| l.id == *label) {
                problems.push(FieldProblem::Label(i));
            }
        }
    }
    if !problems.is_empty() {
        return Err(IssueError::Fields(problems));
    }
    if matches!((draft.start_date, draft.target_date), (Some(start), Some(target)) if start > target) {
        return Err(IssueError::StartAfterTarget);
    }

    // who is assigned: those named who are members of the project (not guests); when none is left, the project's default
    // assignee if they are one
    let members = db.project_members(&scope).await?;
    let assignable = |user: UserId| members.iter().any(|m| m.user_id == user && m.role >= Role::Member);
    let mut assignees: Vec<UserId> = draft.assignee_ids.iter().copied().filter(|u| assignable(*u)).collect();
    if assignees.is_empty() {
        let workspace = authorize_workspace(app, &access.who, &access.slug).await?;
        let default = db.project_detail(&workspace, scope.project_id()).await?.and_then(|d| d.listing.project.default_assignee);
        assignees.extend(default.filter(|u| assignable(*u)));
    }
    assignees.sort();
    assignees.dedup();
    let mut labels = draft.label_ids.clone();
    labels.sort();
    labels.dedup();

    let state = draft.state_id.and_then(|id| states.iter().find(|s| s.id == id)).or_else(|| states.iter().find(|s| s.is_default));
    let now = app.ports.clock.now();
    let attrs = WorkItemAttrs {
        state_id: state.map(|s| s.id),
        priority: draft.priority.unwrap_or(Priority::None),
        start_date: draft.start_date,
        target_date: draft.target_date,
        completed_at: state.filter(|s| s.group == StateGroup::Completed).map(|_| now),
        updated_at: now,
        updated_by: None,
        description_html: draft.description_html.clone().unwrap_or_else(|| "<p></p>".to_string()),
        assignee_ids: assignees,
        label_ids: labels,
    };

    let sort_order = db.max_work_item_sort_order(&scope, attrs.state_id).await?.map_or(FIRST_SORT_ORDER, |greatest| greatest + SORT_ORDER_STEP);

    let mut tx = db.begin(&scope).await?;
    let work = async {
        let alloc = tx.allocate(&scope).await?;
        let item = WorkItem {
            id: WorkItemId(app.ports.ids.new_uuid()),
            workspace_id: scope.workspace_id(),
            project_id: scope.project_id(),
            sequence_id: alloc.sequence_id,
            sort_order,
            name: draft.name.trim().to_string(),
            description: None,
            external_source: None,
            external_id: None,
            created_by: scope.user_id(),
            created_at: now,
            archived_at: None,
            is_draft: false,
            deleted_at: None,
        };
        tx.insert_work_item(&scope, &item).await?;
        tx.insert_work_item_attrs(&scope, item.id, &attrs).await?;
        let row = event(
            app,
            item.workspace_id,
            scope.user_id(),
            "work_item.created",
            item.id,
            serde_json::json!({
                "project_id": item.project_id,
                "sequence_id": item.sequence_id,
                "changed_fields": ["name"],
            }),
        );
        tx.insert_outbox(&scope, &row).await?;
        Ok::<_, DbError>((item, row))
    }
    .await;
    let (item, row) = match work {
        Ok(v) => v,
        Err(e) => {
            // Roll back explicitly: a pooled connection must not be returned mid-transaction.
            let _ = tx.rollback().await;
            return Err(e.into());
        }
    };
    tx.commit().await?;
    relay_after_commit(app, row);
    Ok(WorkItemRow { item, state_group: state.map(|s| s.group), attrs })
}

/// A page of the project's work items (the caller got [`issue_reader`]'s access). A guest sees only what they made when the
/// project does not show guests everything. The page is at most five statements with the caller's membership: a guest's
/// workspace access and the project's flag, and the page (and its count when the page is empty).
pub async fn list_issues(app: &App, access: &IssueAccess, q: IssueListQuery) -> Result<WorkItemPage, IssueError> {
    let created_by = if access.role == Role::Guest && !guest_sees_all(app, access).await? { Some(access.who.user_id) } else { None };
    let query = WorkItemQuery { order: q.order, limit: q.limit, page: q.page, updated_after: q.updated_after, created_by };
    Ok(app.ports.db.work_item_page(&access.scope, &query).await?)
}

/// One work item (the caller got [`issue_reader`]'s access); a guest only for one they made when the project does not show guests
/// everything (`NotAllowedToView`, after the item is found: a work item that is not there is `NotFound` to everyone).
pub async fn get_issue(app: &App, access: &IssueAccess, id: WorkItemId) -> Result<WorkItemRow, IssueError> {
    let row = app.ports.db.work_item_row(&access.scope, id).await?.ok_or(IssueError::NotFound)?;
    if access.role == Role::Guest && row.item.created_by != access.who.user_id && !guest_sees_all(app, access).await? {
        return Err(IssueError::NotAllowedToView);
    }
    Ok(row)
}
