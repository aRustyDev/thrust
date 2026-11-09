//! Work items as the compat plane serves them (issue #44, the work item stage): made, listed a page at a time, opened, changed, and
//! their history read.
//!
//! The rules are the pin's. A work item is made by an admin or a member of the project; a page of them and one of them are read
//! by any member, a guest too, except that a guest of a project that does not show guests everything sees only what they made. What
//! a request names must be there: its state is one of the project's, its assignees are users (and only those who are members of
//! the project, not guests, are kept), its labels are the project's. The wording and the order of a refusal are the compat layer's.
//!
//! A parent, an estimate, a cycle and a module are not built (deviation D-028): a work item has none of them, and a parent that a
//! request names is refused.
//!
//! A work item is changed by its creator or by an admin or a member of the project. What the change records of itself (the history
//! the app shows under the work item) is worked out here, from the row as it was and what the request named, and travels in the
//! payload of the `work_item.updated` event; the activity consumer ([`process_event`](crate::process_event)) writes the rows from
//! it, so the history is a projection that lags the change as the pin's does (a task on its worker), and is written once however
//! often the event is delivered.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
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
    /// A v1 deletion by someone who is neither a project admin nor the work item's maker.
    #[error("only an admin or the creator can delete the work item")]
    OnlyAdminOrCreator,
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
    surface: Surface,
}

/// Which API a caller came through. The two differ in what a work item may be and who sees which (issue #58): the compat plane
/// keeps D-028 (no parent) and hides from a guest what others made (unless the project shows guests everything); the public v1 API
/// accepts a parent and shows every work item to every member of the project, as the pin's views do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Compat,
    V1,
}

impl IssueAccess {
    /// The caller's role in the project.
    pub fn role(&self) -> Role {
        self.role
    }

    /// The proof of the caller's way into the project, for the reads a surface makes beside the use-cases (the states and labels a
    /// body names).
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// Who the caller is.
    pub fn who(&self) -> &Principal {
        &self.who
    }

    /// Whether this is a guest of the compat plane, who may be limited to what they made (see [`guest_sees_all`]).
    fn is_limited_guest(&self) -> bool {
        self.surface == Surface::Compat && self.role == Role::Guest
    }
}

/// May the caller read the project's work items: an admin, a member or a guest of the project (`Forbidden` for anyone else).
pub async fn issue_reader(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
) -> Result<IssueAccess, IssueError> {
    let (scope, role) =
        authorize_with_role(app, who, slug, project, Permission::ReadWorkItems).await.map_err(denied)?;
    Ok(IssueAccess { scope, role, who: who.clone(), slug: slug.clone(), surface: Surface::Compat })
}

/// May the caller read the project's work items through the v1 API: any member of the project, a guest too (`Forbidden` for anyone
/// else). Such an access sees every work item.
pub async fn v1_issue_reader(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
) -> Result<IssueAccess, IssueError> {
    let (scope, role) =
        authorize_with_role(app, who, slug, project, Permission::ReadWorkItems).await.map_err(denied)?;
    Ok(IssueAccess { scope, role, who: who.clone(), slug: slug.clone(), surface: Surface::V1 })
}

/// May the caller make, change and delete work items through the v1 API: an admin or a member of the project (`Forbidden` for a
/// guest, whoever made the work item: the compat plane lets a guest change what they made, the pin's v1 views do not).
pub async fn v1_issue_writer(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
) -> Result<IssueAccess, IssueError> {
    let (scope, role) =
        authorize_with_role(app, who, slug, project, Permission::CreateWorkItem).await.map_err(denied)?;
    Ok(IssueAccess { scope, role, who: who.clone(), slug: slug.clone(), surface: Surface::V1 })
}

/// May the caller make work items in the project: an admin or a member (`Forbidden` for a guest and for anyone else).
pub async fn issue_writer(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
) -> Result<IssueAccess, IssueError> {
    let (scope, role) =
        authorize_with_role(app, who, slug, project, Permission::CreateWorkItem).await.map_err(denied)?;
    Ok(IssueAccess { scope, role, who: who.clone(), slug: slug.clone(), surface: Surface::Compat })
}

/// The work item the caller asked to change, with the proof that they may: an admin or a member of the project may change any, and
/// whoever made a work item may change it, whatever their role (the pin's decorator lets the creator through before it looks at the
/// role). A creator who has left the project is not let in (deviation D-029); a stranger, the project missing and a guest who is not
/// the creator are `Forbidden`, the work item missing is `NotFound` (after the guard, as the pin answers it).
pub async fn issue_to_update(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
    id: WorkItemId,
) -> Result<(IssueAccess, WorkItemRow), IssueError> {
    let access = match authorize_with_role(app, who, slug, project, Permission::UpdateWorkItem).await {
        Ok((scope, role)) => {
            IssueAccess { scope, role, who: who.clone(), slug: slug.clone(), surface: Surface::Compat }
        }
        Err(e) => match denied(e) {
            IssueError::Forbidden => issue_reader(app, who, slug, project).await?,
            other => return Err(other),
        },
    };
    let row = app.ports.db.work_item_row(&access.scope, id).await?;
    match row {
        // a guest is let in for the work items they made, and for no other (nor for one that is not there: the decorator has
        // nothing to say they made)
        Some(row) if access.role >= Role::Member || row.item.created_by == Some(who.user_id) => Ok((access, row)),
        None if access.role >= Role::Member => Err(IssueError::NotFound),
        _ => Err(IssueError::Forbidden),
    }
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
    /// Plane's `point`, 0 to 12 (the request layer checks the range; the database does too).
    pub point: Option<i32>,
    /// The key an integration gave the item. The v1 API sets them (the request layer checks that no other item has the pair); the
    /// compat plane does not.
    pub external_source: Option<String>,
    pub external_id: Option<String>,
}

/// What a page of the project's work items is asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct IssueListQuery {
    pub order: WorkItemOrder,
    pub limit: u32,
    pub page: u32,
    pub updated_after: Option<Timestamp>,
}

/// What a request names that has to be there: a state and a parent as ids, and the people and the labels.
struct Named<'a> {
    state: Option<Uuid>,
    parent: Option<Uuid>,
    assignees: &'a [UserId],
    labels: &'a [Uuid],
}

/// What a request names that is not there, in the order the pin's serializer would find it: the state, the parent (one that exists
/// is not supported on the compat plane), each assignee and each label.
async fn find_problems(
    app: &App,
    scope: &Scope,
    surface: Surface,
    states: &[StateRow],
    named: Named<'_>,
) -> Result<Vec<FieldProblem>, DbError> {
    let Named { state, parent, assignees, labels } = named;
    let db = &app.ports.db;
    let mut problems = Vec::new();
    if state.is_some_and(|id| !states.iter().any(|s| s.id == id)) {
        problems.push(FieldProblem::State);
    }
    if let Some(parent) = parent {
        let there = db.work_item_row(scope, WorkItemId(parent)).await?.is_some();
        // the compat plane has no parent to give (D-028); the v1 API accepts a work item of the project, itself included
        if !there {
            problems.push(FieldProblem::Parent);
        } else if surface == Surface::Compat {
            problems.push(FieldProblem::ParentNotSupported);
        }
    }
    for (i, user) in assignees.iter().enumerate() {
        if db.user_by_id(*user).await?.is_none() {
            problems.push(FieldProblem::Assignee(i));
        }
    }
    if !labels.is_empty() {
        let known = db.project_labels(scope).await?;
        for (i, label) in labels.iter().enumerate() {
            if !known.iter().any(|l| l.id == *label) {
                problems.push(FieldProblem::Label(i));
            }
        }
    }
    Ok(problems)
}

/// Whether a guest of the project sees work items they did not make: the project's `guest_view_all_features`.
async fn guest_sees_all(app: &App, access: &IssueAccess) -> Result<bool, CoreError> {
    let workspace = authorize_workspace(app, &access.who, &access.slug).await?;
    Ok(app
        .ports
        .db
        .project_detail(&workspace, access.scope.project_id())
        .await?
        .is_some_and(|d| d.listing.project.guest_view_all_features))
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
    let named = Named {
        state: draft.state_id,
        parent: draft.parent_id,
        assignees: &draft.assignee_ids,
        labels: &draft.label_ids,
    };
    let problems = find_problems(app, &scope, access.surface, &states, named).await?;
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
        let default =
            db.project_detail(&workspace, scope.project_id()).await?.and_then(|d| d.listing.project.default_assignee);
        assignees.extend(default.filter(|u| assignable(*u)));
    }
    assignees.sort();
    assignees.dedup();
    let mut labels = draft.label_ids.clone();
    labels.sort();
    labels.dedup();

    let state = draft
        .state_id
        .and_then(|id| states.iter().find(|s| s.id == id))
        .or_else(|| states.iter().find(|s| s.is_default));
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
        parent_id: draft.parent_id,
        point: draft.point,
    };

    let sort_order = db
        .max_work_item_sort_order(&scope, attrs.state_id)
        .await?
        .map_or(FIRST_SORT_ORDER, |greatest| greatest + SORT_ORDER_STEP);
    // what the history says of the creation: the item, and each person the request named (not the default assignee, and those the
    // item does not keep as well: the pin records what was asked)
    let mut asked: Vec<UserId> = draft.assignee_ids.clone();
    asked.sort();
    asked.dedup();
    let names = Names::of(app, &states, &[], &asked).await?;
    let history = created_history(&asked, &names);

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
            external_source: draft.external_source.clone(),
            external_id: draft.external_id.clone(),
            created_by: Some(scope.user_id()),
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
                "history": history,
                "epoch": now.timestamp(),
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
    // the pin's v1 queryset leaves out a project that is archived; the compat plane's list does not
    if access.surface == Surface::V1 && app.ports.db.project_is_archived(&access.scope).await? {
        return Ok(WorkItemPage { rows: Vec::new(), has_next: false, total: 0 });
    }
    let created_by =
        if access.is_limited_guest() && !guest_sees_all(app, access).await? { Some(access.who.user_id) } else { None };
    let query =
        WorkItemQuery { order: q.order, limit: q.limit, page: q.page, updated_after: q.updated_after, created_by };
    Ok(app.ports.db.work_item_page(&access.scope, &query).await?)
}

/// One work item (the caller got [`issue_reader`]'s access); a guest only for one they made when the project does not show guests
/// everything (`NotAllowedToView`, after the item is found: a work item that is not there is `NotFound` to everyone).
pub async fn get_issue(app: &App, access: &IssueAccess, id: WorkItemId) -> Result<WorkItemRow, IssueError> {
    if access.surface == Surface::V1 && app.ports.db.project_is_archived(&access.scope).await? {
        return Err(IssueError::NotFound);
    }
    let row = app.ports.db.work_item_row(&access.scope, id).await?.ok_or(IssueError::NotFound)?;
    if access.is_limited_guest()
        && row.item.created_by != Some(access.who.user_id)
        && !guest_sees_all(app, access).await?
    {
        return Err(IssueError::NotAllowedToView);
    }
    Ok(row)
}

// --- what only the v1 API asks of a work item (issue #58) -------------------------------------------------------------------------

/// The work item of the project with this number, as the pin's `issue_objects` finds it (live, and none in an archived project).
pub async fn issue_by_sequence(app: &App, access: &IssueAccess, sequence_id: i64) -> Result<WorkItemRow, IssueError> {
    if app.ports.db.project_is_archived(&access.scope).await? {
        return Err(IssueError::NotFound);
    }
    app.ports.db.work_item_by_sequence(&access.scope, sequence_id).await?.ok_or(IssueError::NotFound)
}

/// The work item of the project that an integration gave this key, as the pin's list answers a request that names both halves.
pub async fn issue_by_external(
    app: &App,
    access: &IssueAccess,
    source: &str,
    id: &str,
) -> Result<WorkItemRow, IssueError> {
    app.ports.db.work_item_by_external(&access.scope, source, id).await?.ok_or(IssueError::NotFound)
}

/// The work item a change or a deletion names, as the pin's `Issue.objects` finds it: live, whatever the project's archive says.
/// (An archived work item and a draft are found there too; this finds neither, which D-033 records.)
pub async fn issue_to_change(app: &App, access: &IssueAccess, id: WorkItemId) -> Result<WorkItemRow, IssueError> {
    app.ports.db.work_item_row(&access.scope, id).await?.ok_or(IssueError::NotFound)
}

/// Search the work items of the workspace's projects that the caller is in, the way the pin's search endpoint does. The text and the
/// numbers it holds are the request layer's to work out; an empty text is no search.
pub async fn search_issues(
    app: &App,
    scope: &WorkspaceScope,
    search: WorkspaceSearch,
) -> Result<Vec<WorkItemHit>, IssueError> {
    if search.text.is_empty() {
        return Ok(Vec::new());
    }
    Ok(app.ports.db.search_workspace_work_items(scope, &search).await?)
}

/// Delete a work item the v1 way: a project admin or the work item's maker (`OnlyAdminOrCreator` for anyone else the project lets
/// write), and with it the work items under it. The deletion and its `work_item.deleted` event are one transaction.
pub async fn delete_issue(app: &App, access: &IssueAccess, id: WorkItemId) -> Result<(), IssueError> {
    let scope = access.scope;
    let row = issue_to_change(app, access, id).await?;
    if row.item.created_by != Some(access.who.user_id) && access.role != Role::Admin {
        return Err(IssueError::OnlyAdminOrCreator);
    }
    let mut tx = app.ports.db.begin(&scope).await?;
    let now = app.ports.clock.now();
    let work = async {
        if !tx.soft_delete_work_item(&scope, id, now).await? {
            return Ok::<_, DbError>(None);
        }
        let row = event(
            app,
            scope.workspace_id(),
            scope.user_id(),
            "work_item.deleted",
            id,
            serde_json::json!({ "project_id": scope.project_id(), "changed_fields": ["deleted_at"] }),
        );
        tx.insert_outbox(&scope, &row).await?;
        Ok(Some(row))
    }
    .await;
    match work {
        Ok(Some(row)) => {
            tx.commit().await?;
            relay_after_commit(app, row);
            Ok(())
        }
        // the work item went between the read and the write
        Ok(None) => {
            let _ = tx.rollback().await;
            Err(IssueError::NotFound)
        }
        Err(e) => {
            let _ = tx.rollback().await;
            Err(e.into())
        }
    }
}

// --- changing a work item (X03) ------------------------------------------------------------------------------------------------

/// One thing a request to change a work item names, read and checked as far as the request alone allows (the compat layer's DRF
/// fields). A request is these in the order it names them, which is the order its history lists them in.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// `sent` is the text the request had (the pin's history quotes it as it came, padding and all); `name` is what is kept of it:
    /// trimmed, 1 to 255 characters.
    Name {
        sent: String,
        name: String,
    },
    /// Already cleaned of what must not be stored.
    Description(String),
    Priority(Priority),
    /// `None` is the project's default state (the pin gives a work item the default when it would have none).
    State(Option<Uuid>),
    /// `sent` is the text the request had (`None` for null): the pin compares it, as text, with the date the work item has.
    Start {
        sent: Option<String>,
        date: Option<NaiveDate>,
    },
    Target {
        sent: Option<String>,
        date: Option<NaiveDate>,
    },
    /// Another work item as the parent, or none (null). The compat plane has none to give (deviation D-028): a work item of the
    /// project is refused as not supported yet there. Either way one that is not there is a [`FieldProblem`].
    Parent(Option<Uuid>),
    /// Plane's `point` (0 to 12), or none; the history records no row of it (D-033).
    Point(Option<i32>),
    /// The key an integration gave the item, a half at a time (the request names the halves it names); the history records no row
    /// of it (D-033). The request layer has checked that no other item has the pair.
    ExternalSource(Option<String>),
    ExternalId(Option<String>),
    /// The people to assign. Those the project does not have as members (guests too) are not assigned, and the history says they
    /// were: the pin records what was asked.
    Assignees(Vec<UserId>),
    Labels(Vec<Uuid>),
}

impl Ask {
    /// The field of the request this is, which the event lists in its `changed_fields`.
    fn field(&self) -> &'static str {
        match self {
            Ask::Name { .. } => "name",
            Ask::Description(_) => "description_html",
            Ask::Priority(_) => "priority",
            Ask::State(_) => "state_id",
            Ask::Start { .. } => "start_date",
            Ask::Target { .. } => "target_date",
            Ask::Parent(_) => "parent_id",
            Ask::Point(_) => "point",
            Ask::ExternalSource(_) => "external_source",
            Ask::ExternalId(_) => "external_id",
            Ask::Assignees(_) => "assignee_ids",
            Ask::Labels(_) => "label_ids",
        }
    }
}

/// A request to change a work item.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IssueUpdate {
    pub asks: Vec<Ask>,
    /// A description written by a migration (the request said `skip_activity` and named a description): the pin records nothing of
    /// it, no history and no event.
    pub silent: bool,
}

/// One change a work item's history records, as the `history` of an event's payload carries it: the pin's `IssueActivity` without
/// the actor, the time and the place, which the envelope and the order of the list give.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryChange {
    /// `created` or `updated`.
    pub verb: String,
    pub field: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub comment: String,
    pub old_identifier: Option<Uuid>,
    pub new_identifier: Option<Uuid>,
}

/// The most of a value (a name, a description, a date) that a history row keeps. The event that carries the row must stay under
/// the queue's message limit (128 KB, ADR-0070), and the app shows none of a description (deviation D-029).
const HISTORY_VALUE_LIMIT: usize = 16 * 1024;

fn capped(mut text: String) -> String {
    // the last character boundary at or before the limit (the whole text when it is shorter)
    text.truncate(text.floor_char_boundary(HISTORY_VALUE_LIMIT));
    text
}

/// The names the history quotes: states, labels and people, by what they are called when the change is made.
#[derive(Debug, Default)]
pub struct Names {
    states: BTreeMap<Uuid, String>,
    labels: BTreeMap<Uuid, String>,
    people: BTreeMap<UserId, String>,
}

impl Names {
    /// The names given: of states, labels and people (a state, label or person not named here is called nothing).
    pub fn new(
        states: impl IntoIterator<Item = (Uuid, String)>,
        labels: impl IntoIterator<Item = (Uuid, String)>,
        people: impl IntoIterator<Item = (UserId, String)>,
    ) -> Names {
        Names {
            states: states.into_iter().collect(),
            labels: labels.into_iter().collect(),
            people: people.into_iter().collect(),
        }
    }

    /// The names of the states and labels given and of `people` (their display names; a person with no profile is called nothing).
    async fn of(app: &App, states: &[StateRow], labels: &[LabelRow], people: &[UserId]) -> Result<Names, DbError> {
        let mut names = Names::new(
            states.iter().map(|s| (s.id, s.name.clone())),
            labels.iter().map(|l| (l.id, l.name.clone())),
            [],
        );
        for user in people {
            let shown = app.ports.db.profile(*user).await?.map(|p| p.display_name).unwrap_or_default();
            names.people.insert(*user, shown);
        }
        Ok(names)
    }

    fn state(&self, id: Option<Uuid>) -> Option<String> {
        id.and_then(|id| self.states.get(&id).cloned())
    }

    fn label(&self, id: Uuid) -> String {
        self.labels.get(&id).cloned().unwrap_or_default()
    }

    fn person(&self, id: UserId) -> String {
        self.people.get(&id).cloned().unwrap_or_default()
    }
}

fn updated(field: &str, comment: &str, old: Option<String>, new: Option<String>) -> HistoryChange {
    HistoryChange {
        verb: "updated".into(),
        field: Some(field.into()),
        old_value: old.map(capped),
        new_value: new.map(capped),
        comment: comment.into(),
        old_identifier: None,
        new_identifier: None,
    }
}

fn added_person(user: UserId, names: &Names) -> HistoryChange {
    HistoryChange {
        new_identifier: Some(user.0),
        ..updated("assignees", "added assignee ", Some(String::new()), Some(names.person(user)))
    }
}

/// What the history says of a work item's creation: that it was made, and that each person the request named was assigned.
fn created_history(asked: &[UserId], names: &Names) -> Vec<HistoryChange> {
    let made = HistoryChange {
        verb: "created".into(),
        field: None,
        old_value: None,
        new_value: None,
        comment: "created the issue".into(),
        old_identifier: None,
        new_identifier: None,
    };
    std::iter::once(made).chain(asked.iter().map(|u| added_person(*u, names))).collect()
}

fn iso(d: Option<NaiveDate>) -> Option<String> {
    d.map(|d| d.format("%Y-%m-%d").to_string())
}

/// What the pin's history records of a change (`issue_activities_task.update_issue_activity`): for each thing the request names,
/// in the order it names them, a row if what it asks differs from what the work item has (the name, the description, the priority,
/// the state, the dates as text), and for the people and the labels a row for each one added and each one dropped, in the order
/// of their ids. The comparison is with what was *asked*, not what was kept: a person the project does not have as a member is
/// recorded as added though they are not, and a date asked in another spelling than the one the work item has is a change.
pub fn update_history(before: &WorkItemRow, asks: &[Ask], names: &Names) -> Vec<HistoryChange> {
    let (item, have) = (&before.item, &before.attrs);
    let mut out = Vec::new();
    for ask in asks {
        match ask {
            Ask::Name { sent, .. } if *sent != item.name => {
                out.push(updated("name", "updated the name to", Some(item.name.clone()), Some(sent.clone())))
            }
            Ask::Description(html) if *html != have.description_html => out.push(updated(
                "description",
                "updated the description to",
                Some(have.description_html.clone()),
                Some(html.clone()),
            )),
            Ask::Priority(p) if *p != have.priority => out.push(updated(
                "priority",
                "updated the priority to",
                Some(have.priority.as_str().into()),
                Some(p.as_str().into()),
            )),
            Ask::State(now) if *now != have.state_id => out.push(HistoryChange {
                old_identifier: have.state_id,
                new_identifier: *now,
                ..updated("state", "updated the state to", names.state(have.state_id), names.state(*now))
            }),
            Ask::Start { sent, .. } if *sent != iso(have.start_date) => out.push(updated(
                "start_date",
                "updated the start date to ",
                Some(iso(have.start_date).unwrap_or_default()),
                Some(sent.clone().unwrap_or_default()),
            )),
            Ask::Target { sent, .. } if *sent != iso(have.target_date) => out.push(updated(
                "target_date",
                "updated the target date to",
                Some(iso(have.target_date).unwrap_or_default()),
                Some(sent.clone().unwrap_or_default()),
            )),
            Ask::Assignees(asked) => {
                let want: BTreeSet<UserId> = asked.iter().copied().collect();
                let had: BTreeSet<UserId> = have.assignee_ids.iter().copied().collect();
                out.extend(want.difference(&had).map(|u| added_person(*u, names)));
                out.extend(had.difference(&want).map(|u| HistoryChange {
                    old_identifier: Some(u.0),
                    ..updated("assignees", "removed assignee ", Some(names.person(*u)), Some(String::new()))
                }));
            }
            Ask::Labels(asked) => {
                let want: BTreeSet<Uuid> = asked.iter().copied().collect();
                let had: BTreeSet<Uuid> = have.label_ids.iter().copied().collect();
                out.extend(want.difference(&had).map(|l| HistoryChange {
                    new_identifier: Some(*l),
                    ..updated("labels", "added label ", Some(String::new()), Some(names.label(*l)))
                }));
                out.extend(had.difference(&want).map(|l| HistoryChange {
                    old_identifier: Some(*l),
                    ..updated("labels", "removed label ", Some(names.label(*l)), Some(String::new()))
                }));
            }
            _ => {}
        }
    }
    out
}

/// Change a work item (the caller got [`issue_to_update`]'s access and row). What the request names must be there ([`FieldProblem`]),
/// and the start must not be after the target, counting the date the work item has when the request names one only (the pin
/// accepts an inverted pair then; deviation D-029). The state, when named, is changed to; a work item that would have none gets the
/// project's default (`Issue.save`); `completed_at` is set when the state changes to one of the `completed` group and cleared when
/// it changes to another. The people are those the request names who are members of the project, not guests, replacing whoever
/// was assigned. The change, and the event that carries its history, are one transaction.
pub async fn update_issue(
    app: &App,
    access: &IssueAccess,
    before: WorkItemRow,
    update: IssueUpdate,
) -> Result<(), IssueError> {
    let scope = access.scope;
    let db = &app.ports.db;
    let id = before.item.id;

    let mut name = before.item.name.clone();
    let mut html = before.attrs.description_html.clone();
    let mut priority = before.attrs.priority;
    let (mut start, mut target) = (before.attrs.start_date, before.attrs.target_date);
    let mut state_asked: Option<Option<Uuid>> = None;
    let (mut parent, mut point) = (before.attrs.parent_id, before.attrs.point);
    let mut parent_asked: Option<Uuid> = None;
    let (mut external_source, mut external_id) = (before.item.external_source.clone(), before.item.external_id.clone());
    let (mut assignees, mut labels): (Option<Vec<UserId>>, Option<Vec<Uuid>>) = (None, None);
    for ask in &update.asks {
        match ask {
            Ask::Name { name: kept, .. } => name = kept.clone(),
            Ask::Description(h) => html = h.clone(),
            Ask::Priority(p) => priority = *p,
            Ask::State(s) => state_asked = Some(*s),
            Ask::Start { date, .. } => start = *date,
            Ask::Target { date, .. } => target = *date,
            Ask::Parent(p) => {
                parent = *p;
                parent_asked = *p;
            }
            Ask::Point(p) => point = *p,
            Ask::ExternalSource(s) => external_source = s.clone(),
            Ask::ExternalId(i) => external_id = i.clone(),
            Ask::Assignees(a) => assignees = Some(a.clone()),
            Ask::Labels(l) => labels = Some(l.clone()),
        }
    }
    crate::validate_name(&name)?;

    let states = db.project_states(&scope).await?;
    let named = Named {
        state: state_asked.flatten(),
        parent: parent_asked,
        assignees: assignees.as_deref().unwrap_or_default(),
        labels: labels.as_deref().unwrap_or_default(),
    };
    let problems = find_problems(app, &scope, access.surface, &states, named).await?;
    if !problems.is_empty() {
        return Err(IssueError::Fields(problems));
    }
    if matches!((start, target), (Some(a), Some(b)) if a > b) {
        return Err(IssueError::StartAfterTarget);
    }

    let wanted = state_asked.unwrap_or(before.attrs.state_id);
    let state = match wanted {
        Some(id) => states.iter().find(|s| s.id == id),
        None => states.iter().find(|s| s.is_default).or_else(|| states.first()),
    };
    let state_id = state.map(|s| s.id);
    let now = app.ports.clock.now();
    let completed_at = if state_id != before.attrs.state_id {
        state.filter(|s| s.group == StateGroup::Completed).map(|_| now)
    } else {
        before.attrs.completed_at
    };

    let assignees = match assignees {
        Some(asked) => {
            let members = db.project_members(&scope).await?;
            let mut kept: Vec<UserId> = asked
                .iter()
                .copied()
                .filter(|u| members.iter().any(|m| m.user_id == *u && m.role >= Role::Member))
                .collect();
            kept.sort();
            kept.dedup();
            Some(kept)
        }
        None => None,
    };
    let kept_labels = labels.clone().map(|mut l| {
        l.sort();
        l.dedup();
        l
    });

    // what the history says, worked out from the row as it was (not when the request said not to keep one)
    let history = if update.silent {
        None
    } else {
        let mut people: BTreeSet<UserId> = BTreeSet::new();
        for ask in &update.asks {
            if let Ask::Assignees(asked) = ask {
                people.extend(asked.iter().copied());
                people.extend(before.attrs.assignee_ids.iter().copied());
            }
        }
        let known_labels = if labels.is_some() { db.project_labels(&scope).await? } else { Vec::new() };
        let people: Vec<UserId> = people.into_iter().collect();
        let names = Names::of(app, &states, &known_labels, &people).await?;
        Some(update_history(&before, &update.asks, &names))
    };

    let change = WorkItemUpdate {
        name,
        state_id,
        priority,
        start_date: start,
        target_date: target,
        completed_at,
        updated_at: now,
        updated_by: access.who.user_id,
        description_html: html,
        assignee_ids: assignees,
        label_ids: kept_labels,
        external_source,
        external_id,
        parent_id: parent,
        point,
    };
    let mut tx = db.begin(&scope).await?;
    let work = async {
        if !tx.update_work_item(&scope, id, &change).await? {
            return Ok(None);
        }
        let row = match &history {
            Some(history) => {
                let row = event(
                    app,
                    scope.workspace_id(),
                    scope.user_id(),
                    "work_item.updated",
                    id,
                    serde_json::json!({
                        "project_id": scope.project_id(),
                        "sequence_id": before.item.sequence_id,
                        "changed_fields": update.asks.iter().map(Ask::field).collect::<Vec<_>>(),
                        "history": history,
                        "epoch": now.timestamp(),
                    }),
                );
                tx.insert_outbox(&scope, &row).await?;
                Some(row)
            }
            None => None,
        };
        Ok::<_, DbError>(Some(row))
    }
    .await;
    match work {
        Ok(Some(row)) => {
            tx.commit().await?;
            if let Some(row) = row {
                relay_after_commit(app, row);
            }
            Ok(())
        }
        // the work item went between the read and the write
        Ok(None) => {
            let _ = tx.rollback().await;
            Err(IssueError::NotFound)
        }
        Err(e) => {
            // Roll back explicitly: a pooled connection must not be returned mid-transaction.
            let _ = tx.rollback().await;
            Err(e.into())
        }
    }
}

// --- the history (X02) ------------------------------------------------------------------------------------------------------------

/// The consumer's half: write the rows of the `history` an event carries (a `work_item.created` or `work_item.updated` event that
/// names none writes none). A change to a description is not a row of its own when the work item's newest row is a change to its
/// description by the same person: the pin moves that row's time instead, so typing in the editor is one change, not hundreds.
/// Idempotent: a row is written once per (event, place), and a redelivery moves the same row to the same time.
pub(crate) async fn project_history(app: &App, env: &Envelope) -> Result<(), CoreError> {
    if env.entity_type != "work_item" {
        return Ok(());
    }
    let (Some(project), Some(history)) = (env.project_id(), env.payload.get("history")) else { return Ok(()) };
    // an event this code wrote always parses; one that does not is no history rather than a message that is retried for ever
    let Ok(changes) = serde_json::from_value::<Vec<HistoryChange>>(history.clone()) else { return Ok(()) };
    let db = &app.ports.db;
    let item = WorkItemId(env.entity_id);
    let epoch =
        env.payload.get("epoch").and_then(serde_json::Value::as_i64).unwrap_or_else(|| env.occurred_at.timestamp());
    let mut rows = Vec::new();
    for (place, change) in changes.into_iter().enumerate() {
        if change.field.as_deref() == Some("description") {
            if let Some(last) = db.latest_work_item_activity(&app.system, env.workspace_id, project, item).await? {
                if last.field.as_deref() == Some("description") && last.actor_id == env.actor_id {
                    db.touch_work_item_activity(&app.system, last.id, env.occurred_at).await?;
                    continue;
                }
            }
        }
        rows.push(WorkItemActivity {
            id: ActivityId(app.ports.ids.new_uuid()),
            event_id: env.event_id,
            ordinal: place as i32,
            workspace_id: env.workspace_id,
            project_id: project,
            work_item_id: item,
            actor_id: env.actor_id,
            verb: change.verb,
            field: change.field,
            old_value: change.old_value,
            new_value: change.new_value,
            comment: change.comment,
            old_identifier: change.old_identifier,
            new_identifier: change.new_identifier,
            epoch,
            created_at: env.occurred_at,
        });
    }
    if !rows.is_empty() {
        db.insert_work_item_activities(&app.system, &rows).await?;
    }
    Ok(())
}

/// A work item's history as the app asks for it: the rows, oldest first, with the work item as it is now and the people who made
/// the changes.
#[derive(Debug, Clone)]
pub struct History {
    pub rows: Vec<WorkItemActivity>,
    /// The work item as it is now. Rows are only returned for a work item that is there.
    pub item: Option<WorkItemRow>,
    pub people: BTreeMap<UserId, UserProfile>,
    /// The project and the workspace the rows are in, which each row of the app's answer repeats.
    pub project: Option<ProjectRow>,
    pub workspace: Option<Workspace>,
}

/// The history of a work item (the caller got [`issue_reader`]'s access), those rows made after `after` when it is given. A work
/// item that is not there has none (the pin answers an empty list, not a 404): any member of the project may read any work item's
/// history, a guest too and whether or not the project shows guests everything, as the pin lets them.
pub async fn issue_history(
    app: &App,
    access: &IssueAccess,
    id: WorkItemId,
    after: Option<Timestamp>,
) -> Result<History, IssueError> {
    let db = &app.ports.db;
    let rows = db.work_item_history(&access.scope, id, after).await?;
    let empty = History { rows: Vec::new(), item: None, people: BTreeMap::new(), project: None, workspace: None };
    if rows.is_empty() {
        return Ok(empty);
    }
    let Some(item) = db.work_item_row(&access.scope, id).await? else { return Ok(empty) };
    let mut people = BTreeMap::new();
    for actor in rows.iter().filter_map(|r| r.actor_id).collect::<BTreeSet<_>>() {
        if let Some(profile) = db.profile(actor).await? {
            people.insert(actor, profile);
        }
    }
    let workspace_scope = authorize_workspace(app, &access.who, &access.slug).await?;
    let project = db.project_detail(&workspace_scope, access.scope.project_id()).await?.map(|d| d.listing.project);
    let workspace = db
        .workspaces_of(access.who.user_id)
        .await?
        .into_iter()
        .map(|l| l.workspace)
        .find(|w| w.id == access.scope.workspace_id());
    Ok(History { rows, item: Some(item), people, project, workspace })
}

/// Whether the work item is there (live, in the project), for a route that answers a 404 for one that is not and shows nothing of it.
pub async fn issue_there(app: &App, access: &IssueAccess, id: WorkItemId) -> Result<bool, IssueError> {
    Ok(app.ports.db.work_item_row(&access.scope, id).await?.is_some())
}
