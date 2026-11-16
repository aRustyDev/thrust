//! The body of the label routes: the pin's `Label` (`LabelSerializer`, `fields = "__all__"`).
//!
//! As in `states.rs`, the `schema` attributes say what a field's Rust type cannot, and each is something the routes guarantee: a text
//! no longer than the bound its request was checked against, a time that is `to_rfc3339_opts`.

use serde::Serialize;
use thrust_contracts::Uuid;

/// The pin's `LabelSerializer`: 15 keys, all of them in every body (a key whose value can be `null` is `Option`, which is what the
/// document says of it). A work item that is read with `expand=labels` carries this body too (`work_items::expansion`).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub(crate) struct LabelDto {
    pub(crate) id: Uuid,
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) created_at: String,
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) updated_at: String,
    /// Always `null`: a deleted label is not shown.
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) deleted_at: Option<String>,
    pub(crate) created_by: Option<Uuid>,
    pub(crate) updated_by: Option<Uuid>,
    pub(crate) workspace: Uuid,
    pub(crate) project: Uuid,
    /// A live label of the same project, or none (D-032 (1)).
    pub(crate) parent: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) name: String,
    pub(crate) description: String,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) color: String,
    pub(crate) sort_order: f64,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) external_source: Option<String>,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) external_id: Option<String>,
}
