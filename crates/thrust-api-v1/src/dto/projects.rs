//! The bodies of the project routes: the pin's `Project` (`ProjectSerializer`) and `ProjectLite` (`ProjectLiteSerializer`).
//!
//! The `schema` attributes say what a field's Rust type cannot, and each is something a route guarantees: a text no longer than the
//! bound its request was checked against, a time that is `to_rfc3339_opts`, a number of months the request kept to 0 to 12.

use serde::Serialize;
use serde_json::Value;
use thrust_contracts::Uuid;

/// The pin's `ProjectSerializer`: 43 keys, and `sort_order` besides when the caller's place in the list is part of the answer. The pin
/// counts cycles and modules; Thrust has neither, so both are 0 (a deviation the register carries, D-030 (1)), and no deploy board
/// exists. A key whose value can be `null` here is `Option`, which is what the document says of it (the pin's schema calls some of
/// these non-nullable and sends `null` all the same: EX-006).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub(crate) struct ProjectDto {
    pub(crate) id: Uuid,
    pub(crate) workspace: Uuid,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) name: String,
    #[cfg_attr(feature = "openapi", schema(max_length = 12))]
    pub(crate) identifier: String,
    pub(crate) description: String,
    pub(crate) description_text: Option<Value>,
    pub(crate) description_html: Option<Value>,
    #[cfg_attr(feature = "openapi", schema(schema_with = crate::openapi::network))]
    pub(crate) network: u8,
    pub(crate) emoji: Option<String>,
    pub(crate) icon_prop: Option<Value>,
    pub(crate) logo_props: Value,
    pub(crate) cover_image: Option<String>,
    pub(crate) cover_image_url: Option<String>,
    pub(crate) cover_image_asset: Option<Uuid>,
    pub(crate) default_assignee: Option<Uuid>,
    pub(crate) project_lead: Option<Uuid>,
    pub(crate) module_view: bool,
    pub(crate) cycle_view: bool,
    pub(crate) issue_views_view: bool,
    pub(crate) page_view: bool,
    pub(crate) intake_view: bool,
    pub(crate) is_time_tracking_enabled: bool,
    pub(crate) is_issue_type_enabled: bool,
    pub(crate) guest_view_all_features: bool,
    pub(crate) estimate: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(schema_with = crate::openapi::months))]
    pub(crate) archive_in: i32,
    #[cfg_attr(feature = "openapi", schema(schema_with = crate::openapi::months))]
    pub(crate) close_in: i32,
    pub(crate) default_state: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) archived_at: Option<String>,
    #[cfg_attr(feature = "openapi", schema(schema_with = crate::openapi::timezone))]
    pub(crate) timezone: String,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) external_source: Option<String>,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) external_id: Option<String>,
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) created_at: String,
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) updated_at: String,
    pub(crate) created_by: Option<Uuid>,
    pub(crate) updated_by: Option<Uuid>,
    /// Always `null`: a deleted project is not shown.
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) deleted_at: Option<String>,
    pub(crate) is_member: bool,
    /// `null` for a project the caller sees only because it is public.
    #[cfg_attr(feature = "openapi", schema(schema_with = crate::openapi::nullable_whole_number))]
    pub(crate) member_role: Option<u8>,
    pub(crate) total_members: i64,
    pub(crate) total_cycles: i64,
    pub(crate) total_modules: i64,
    pub(crate) is_deployed: bool,
    /// Present in a list and absent from one project: the caller's place among the project's members, `null` for a person who has
    /// none (the outer `Option` is whether the key is there, the inner one what it holds).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<f64>))]
    pub(crate) sort_order: Option<Option<f64>>,
}

/// The pin's `ProjectLiteSerializer`: nine keys.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub(crate) struct ProjectLiteDto {
    pub(crate) id: Uuid,
    #[cfg_attr(feature = "openapi", schema(max_length = 12))]
    pub(crate) identifier: String,
    #[cfg_attr(feature = "openapi", schema(max_length = 255))]
    pub(crate) name: String,
    pub(crate) cover_image: Option<String>,
    pub(crate) icon_prop: Option<Value>,
    pub(crate) emoji: Option<String>,
    pub(crate) description: String,
    pub(crate) cover_image_url: Option<String>,
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub(crate) archived_at: Option<String>,
}
