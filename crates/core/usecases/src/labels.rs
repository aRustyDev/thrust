//! The labels of a project as Plane's v1 API has them (issue #58): listed, opened, made, changed and deleted.
//!
//! Who may write is the caller's to decide when it makes the [`Scope`]: [`crate::Permission::WriteLabel`] is for a project admin or
//! member, and a read is for any member. What the pin does is reproduced as it is, including its order of checks, which differs from
//! the states':
//!
//! * the pin's queryset is the same for a read, a change and a deletion: the project's live labels, none of them in a project that is
//!   archived (a state is found by a change and a deletion whatever the archive says; a label is not);
//! * a creation checks an integration's key that another label has, then writes; a change finds its label first, then checks the key,
//!   which is refused only when the request names both halves of it, and which leaves out the label itself;
//! * a new label takes Plane's place (the database does that), and a name that a live label has is refused by the database's unique
//!   index;
//! * a deletion takes the labels under the label with it, and the work items lose them (the database does that, in one statement).
//!
//! One departure, recorded as D-032: a label's parent is a live label of the same project. The pin's field accepts any label of any
//! project (and workspace), which would let a request name, and learn of, labels it cannot see.

use thiserror::Error;
use thrust_contracts::*;
use thrust_ports::*;

use crate::{App, CoreError};

/// The place Plane's model gives a label that nothing else places (`Label.sort_order`'s default): the first label of a project has it.
pub const FIRST_LABEL_SORT_ORDER: f64 = 65535.0;

/// Why a label's use-case refused.
#[derive(Debug, Error)]
pub enum LabelError {
    /// No such label in the project, as the pin's queryset has it.
    #[error("no such label")]
    NotFound,
    /// A label of the project already has this name. `id` is the one the caller can name, if it is found.
    #[error("a label of this name exists")]
    NameTaken { id: Option<Uuid> },
    /// Another label of the project has this integration key; `id` is the label the pin names in its answer.
    #[error("a label with this external key exists")]
    ExternalTaken { id: Uuid },
    /// The parent a request names is no live label of the project.
    #[error("no such parent label")]
    NoSuchParent(Uuid),
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl From<DbError> for LabelError {
    fn from(e: DbError) -> Self {
        LabelError::Core(e.into())
    }
}

/// What a request may set of a label, each only when it names it. A field that holds an `Option` says whether the request names it
/// (the outer one) and what it sets it to (the inner, `None` for null).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LabelPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub sort_order: Option<f64>,
    pub parent: Option<Option<Uuid>>,
    pub external_source: Option<Option<String>>,
    pub external_id: Option<Option<String>>,
}

/// The integration key a request names, when it names both halves of it and neither is empty (the pin's `request.data.get(...)` of each
/// is truthy).
fn named_key(source: &Option<Option<String>>, id: &Option<Option<String>>) -> Option<(String, String)> {
    let half = |h: &Option<Option<String>>| h.as_ref().and_then(|v| v.clone()).filter(|v| !v.is_empty());
    Some((half(source)?, half(id)?))
}

/// The label of the project, other than `except`, that has this key, if there is one.
fn with_key<'a>(labels: &'a [LabelRow], except: Option<Uuid>, source: &str, key: &str) -> Option<&'a LabelRow> {
    labels.iter().find(|o| {
        Some(o.id) != except && o.external_source.as_deref() == Some(source) && o.external_id.as_deref() == Some(key)
    })
}

/// What a read of the project's labels shows: the live ones, newest first (the pin's `-created_at`; labels made in the same instant
/// come the way they were placed, the one placed last first, then by id); none at all in a project that is archived.
pub async fn visible_labels(app: &App, scope: &Scope) -> Result<Vec<LabelRow>, LabelError> {
    if app.ports.db.project_is_archived(scope).await? {
        return Ok(Vec::new());
    }
    let mut labels = app.ports.db.project_labels(scope).await?;
    labels.sort_by(|a, b| {
        b.created_at.cmp(&a.created_at).then(b.sort_order.total_cmp(&a.sort_order)).then(a.id.cmp(&b.id))
    });
    Ok(labels)
}

/// One label as the pin's queryset finds it, for a read, a change or a deletion alike. `NotFound` for any other, so a request about
/// one is answered before its body's fields are looked at.
pub async fn visible_label(app: &App, scope: &Scope, id: Uuid) -> Result<LabelRow, LabelError> {
    visible_labels(app, scope).await?.into_iter().find(|l| l.id == id).ok_or(LabelError::NotFound)
}

/// Whether `id` is a live label of the scope's project, which is what a label's parent must be.
pub async fn is_label_of_project(app: &App, scope: &Scope, id: Uuid) -> Result<bool, LabelError> {
    Ok(app.ports.db.project_labels(scope).await?.iter().any(|l| l.id == id))
}

/// Make a label. It needs a name; its colour and description are empty unless it says; its place is the database's; it has no parent
/// unless it names one of the project's labels.
pub async fn create_label(app: &App, scope: &Scope, patch: LabelPatch) -> Result<LabelRow, LabelError> {
    let LabelPatch { name, description, color, sort_order, parent, external_source, external_id } = patch;
    let Some(name) = name else {
        return Err(CoreError::Invalid("a label needs a name".into()).into());
    };
    let others = app.ports.db.project_labels(scope).await?;
    let parent_id = parent.flatten();
    if let Some(p) = parent_id.filter(|p| !others.iter().any(|o| o.id == *p)) {
        return Err(LabelError::NoSuchParent(p));
    }
    // the pin refuses a key only when the request names both halves of it
    if let Some((source, key)) = named_key(&external_source, &external_id) {
        if let Some(taken) = with_key(&others, None, &source, &key) {
            return Err(LabelError::ExternalTaken { id: taken.id });
        }
    }
    let now = app.ports.clock.now();
    let row = LabelRow {
        id: app.ports.ids.new_uuid(),
        workspace_id: scope.workspace_id(),
        project_id: scope.project_id(),
        parent_id,
        name,
        description: description.unwrap_or_default(),
        color: color.unwrap_or_default(),
        sort_order: sort_order.unwrap_or(FIRST_LABEL_SORT_ORDER),
        created_by: Some(scope.user_id()),
        created_at: now,
        updated_at: now,
        external_source: external_source.flatten(),
        external_id: external_id.flatten(),
        updated_by: None,
    };
    match app.ports.db.insert_label(scope, &row).await {
        Ok(stored) => Ok(stored),
        Err(DbError::Conflict(_)) => match others.iter().find(|o| o.name == row.name) {
            Some(o) => Err(LabelError::NameTaken { id: Some(o.id) }),
            None => Err(CoreError::Internal("a new label's name is taken, and no label has it".into()).into()),
        },
        Err(e) => Err(e.into()),
    }
}

/// Change a label: the fields the patch names are set; who changed it and when are saved with it.
pub async fn change_label(app: &App, scope: &Scope, id: Uuid, patch: LabelPatch) -> Result<LabelRow, LabelError> {
    let current = visible_label(app, scope, id).await?;
    let others = app.ports.db.project_labels(scope).await?;
    if let Some(p) = patch.parent.flatten().filter(|p| !others.iter().any(|o| o.id == *p)) {
        return Err(LabelError::NoSuchParent(p));
    }
    // a key that another label has, named in both halves; the pin names the label that is being changed in its answer, not the one
    // that has the key
    if let Some((source, key)) = named_key(&patch.external_source, &patch.external_id) {
        if with_key(&others, Some(id), &source, &key).is_some() {
            return Err(LabelError::ExternalTaken { id: current.id });
        }
    }

    let mut row = current;
    if let Some(v) = patch.name {
        row.name = v;
    }
    if let Some(v) = patch.description {
        row.description = v;
    }
    if let Some(v) = patch.color {
        row.color = v;
    }
    if let Some(v) = patch.sort_order {
        row.sort_order = v;
    }
    if let Some(v) = patch.parent {
        row.parent_id = v;
    }
    if let Some(v) = patch.external_source {
        row.external_source = v;
    }
    if let Some(v) = patch.external_id {
        row.external_id = v;
    }
    row.updated_by = Some(scope.user_id());
    row.updated_at = app.ports.clock.now();
    match app.ports.db.update_label(scope, &row).await {
        Ok(true) => {}
        Ok(false) => return Err(LabelError::NotFound),
        Err(DbError::Conflict(_)) => {
            let other = others.iter().find(|o| o.id != id && o.name == row.name).map(|o| o.id);
            return Err(LabelError::NameTaken { id: other });
        }
        Err(e) => return Err(e.into()),
    }
    app.ports.db.project_labels(scope).await?.into_iter().find(|l| l.id == id).ok_or(LabelError::NotFound)
}

/// Delete a label, with the labels under it, and take it from the work items that carry it.
pub async fn delete_label(app: &App, scope: &Scope, id: Uuid) -> Result<(), LabelError> {
    visible_label(app, scope, id).await?;
    if !app.ports.db.delete_label(scope, id, app.ports.clock.now()).await? {
        return Err(LabelError::NotFound);
    }
    Ok(())
}
