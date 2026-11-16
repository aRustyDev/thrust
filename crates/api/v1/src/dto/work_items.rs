//! The bodies of the work item routes: the pin's `Issue` (`IssueSerializer`), what `expand` swaps its people, state and labels for,
//! and what the workspace search answers with.
//!
//! As in `states.rs`, the `schema` attributes say what a field's Rust type cannot, and each is something the routes guarantee: a
//! text no longer than the bound its request was checked against, a point that the database keeps between 0 and 12, a date or a time
//! that is written as the pin writes it. Where a body can say two things, the type says both: a `state`, an assignee or a label is its
//! id, or with `expand` the object (the pin's own schema says only the id, and its routes answer the object too).

use serde::Serialize;
use thrust_contracts::{Priority, Uuid};

use super::labels::LabelDto;

/// The pin's `PriorityEnum`: the five priorities, as the pin writes them and in its order. The request descriptions use it too, so the
/// five names are written once.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub(crate) enum PriorityEnum {
    Urgent,
    High,
    Medium,
    Low,
    None,
}

impl From<Priority> for PriorityEnum {
    fn from(p: Priority) -> Self {
        match p {
            Priority::Urgent => Self::Urgent,
            Priority::High => Self::High,
            Priority::Medium => Self::Medium,
            Priority::Low => Self::Low,
            Priority::None => Self::None,
        }
    }
}

/// A state as `expand=state` shows it: the four keys the oracle's corpus has.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub(crate) struct StateBriefDto {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) color: String,
    #[cfg_attr(feature = "openapi", schema(schema_with = crate::openapi::state_group))]
    pub(crate) group: &'static str,
}

/// A person as `expand=assignees` shows them (the pin's `UserLiteSerializer`).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub(crate) struct UserBriefDto {
    pub(crate) id: Uuid,
    pub(crate) first_name: String,
    pub(crate) last_name: String,
    pub(crate) display_name: String,
    pub(crate) email: Option<String>,
    pub(crate) avatar: String,
    /// `null` for a person with no avatar.
    pub(crate) avatar_url: Option<String>,
}

/// A work item's state: its id, or with `expand=state` the state.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub(crate) enum StateRef {
    Id(Uuid),
    Expanded(StateBriefDto),
}

/// An assignee: the person's id, or with `expand=assignees` the person.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub(crate) enum UserRef {
    Id(Uuid),
    Expanded(UserBriefDto),
}

/// A label: its id, or with `expand=labels` the label as the label routes answer it.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
#[allow(
    clippy::large_enum_variant,
    reason = "built once per label of a row and serialized at once: boxing the object would allocate to save a few hundred bytes of stack"
)]
pub(crate) enum LabelRef {
    Id(Uuid),
    Expanded(LabelDto),
}

/// The pin's `IssueSerializer`: 29 keys, all of them in every body. A work item that is read is live, so it is not a draft, not archived
/// and not deleted, and it has no estimate point and no type (D-033). A key whose value can be `null` is `Option`, which is what the
/// document says of it.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub(crate) struct IssueDto {
    pub(crate) id: Uuid,
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) created_at: String,
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) updated_at: String,
    /// Always `null`: a deleted work item is not shown.
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) deleted_at: Option<String>,
    pub(crate) created_by: Option<Uuid>,
    pub(crate) updated_by: Option<Uuid>,
    pub(crate) workspace: Uuid,
    pub(crate) project: Uuid,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) name: String,
    pub(crate) description_html: String,
    /// Always `null`: the pin's binary form of the description is not kept.
    #[cfg_attr(feature = "openapi", schema(format = Byte))]
    pub(crate) description_binary: Option<String>,
    #[cfg_attr(feature = "openapi", schema(schema_with = crate::openapi::priority))]
    pub(crate) priority: PriorityEnum,
    #[cfg_attr(feature = "openapi", schema(format = Date))]
    pub(crate) start_date: Option<String>,
    #[cfg_attr(feature = "openapi", schema(format = Date))]
    pub(crate) target_date: Option<String>,
    /// The project's running number ("PLATF-7" is 7). A bigint here; the pin's column is an int32, so its schema bounds it and this
    /// one does not (the register says so).
    pub(crate) sequence_id: i64,
    pub(crate) sort_order: f64,
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) completed_at: Option<String>,
    /// Always `null`: a list is of work items that are not archived.
    #[cfg_attr(feature = "openapi", schema(format = Date))]
    pub(crate) archived_at: Option<String>,
    /// Always `false`: a draft is not shown.
    pub(crate) is_draft: bool,
    pub(crate) state: Option<StateRef>,
    pub(crate) parent: Option<Uuid>,
    /// A whole number from 0 to 12, which the database checks.
    #[cfg_attr(feature = "openapi", schema(schema_with = crate::openapi::point))]
    pub(crate) point: Option<i32>,
    /// Always `null`: there are no estimates.
    pub(crate) estimate_point: Option<Uuid>,
    /// Always `null`: there are no work item types.
    #[serde(rename = "type")]
    pub(crate) type_: Option<Uuid>,
    /// Always `null`, as `type`.
    pub(crate) type_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) external_source: Option<String>,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) external_id: Option<String>,
    pub(crate) assignees: Vec<UserRef>,
    pub(crate) labels: Vec<LabelRef>,
}

/// What a list answers: a page, or, when the request names an integration key (both halves), the one work item that has it. The pin's
/// schema describes only the page.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub(crate) enum Listing<T> {
    Page(crate::pagination::Page<T>),
    One(T),
}

/// One hit of the workspace search: the pin's `IssueSearch`, which its schema describes as the whole answer.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub(crate) struct IssueSearchItemDto {
    pub(crate) name: String,
    pub(crate) id: Uuid,
    /// A whole number here and on the pin's wire (the oracle's corpus, `search`); the pin's schema says a string.
    pub(crate) sequence_id: i64,
    #[serde(rename = "project__identifier")]
    pub(crate) project_identifier: String,
    pub(crate) project_id: Uuid,
    #[serde(rename = "workspace__slug")]
    pub(crate) workspace_slug: String,
}

/// What the workspace search answers: `{"issues": [...]}`. The pin's schema describes one hit as the whole answer; its recorded answers
/// are this (the oracle's corpus, `search`).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub(crate) struct IssueSearchDto {
    pub(crate) issues: Vec<IssueSearchItemDto>,
}
