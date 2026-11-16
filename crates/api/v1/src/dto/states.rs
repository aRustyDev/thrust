//! The body of the state routes: the pin's `State` (`StateSerializer`).
//!
//! As in `projects.rs`, the `schema` attributes say what a field's Rust type cannot, and each is something the routes guarantee: a
//! text no longer than the bound its request was checked against, a time that is `to_rfc3339_opts`, a group that is one of
//! `StateGroup`'s six.

use serde::Serialize;
use thrust_contracts::Uuid;

/// The pin's `StateSerializer`: 18 keys. `is_triage` is whether the group is the intake's, which a state of the list never is (the pin
/// stores it apart from the group, and a request can set it on a state of another group: the register carries that). A key whose value
/// can be `null` here is `Option`, which is what the document says of it.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub(crate) struct StateDto {
    pub(crate) id: Uuid,
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) created_at: String,
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) updated_at: String,
    /// Always `null`: a deleted state is not shown.
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) deleted_at: Option<String>,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) name: String,
    pub(crate) description: String,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) color: String,
    /// The pin's `slugify` of the name ([`crate::states::slugify`]): it can be empty, for a name with no letter or digit that NFKD
    /// leaves in ASCII, which is what the pin answers too.
    pub(crate) slug: String,
    pub(crate) sequence: f64,
    #[cfg_attr(feature = "openapi", schema(schema_with = crate::openapi::state_group))]
    pub(crate) group: &'static str,
    pub(crate) is_triage: bool,
    pub(crate) default: bool,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) external_source: Option<String>,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) external_id: Option<String>,
    pub(crate) created_by: Option<Uuid>,
    pub(crate) updated_by: Option<Uuid>,
    pub(crate) project: Uuid,
    pub(crate) workspace: Uuid,
}
