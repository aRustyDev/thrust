//! `/api/instances/`: what the React app reads before anyone signs in.

use axum::http::header::HOST;
use axum::http::HeaderMap;
use axum::{Extension, Json};
use serde::Serialize;
use thrust_contracts::chrono::SecondsFormat;
use thrust_core::App;

use crate::PlaneError;

/// Shown in the admin app and the footer; the pin's own default is "Plane Community Edition".
const INSTANCE_NAME: &str = "Thrust";
/// One Thrust deployment is one instance. Fixed, like Plane's singleton `Instance` row id is for its lifetime.
const INSTANCE_ROW_ID: &str = "00000000-0000-7000-8000-000000000001";
const INSTANCE_ID: &str = "7468727573740000000000a1";
/// The pin's `instance_changelog_url`; a constant of the pinned Plane, not of a deployment.
const CHANGELOG_URL: &str = "https://sites.plane.so/pages/691ef037bcfe416a902e48cb55f59891/";

/// Plane's `/api/instances/` answer. Field names and types are the pin's (fixture S01); `file_size_limit` is a float
/// there, so it is one here.
#[derive(Serialize)]
pub(crate) struct InstanceResponse {
    config: Config,
    instance: Instance,
}

#[derive(Serialize)]
struct Config {
    enable_signup: bool,
    is_workspace_creation_disabled: bool,
    is_google_enabled: bool,
    is_github_enabled: bool,
    is_gitlab_enabled: bool,
    is_gitea_enabled: bool,
    is_oidc_enabled: bool,
    is_magic_login_enabled: bool,
    is_email_password_enabled: bool,
    github_app_name: &'static str,
    slack_client_id: Option<&'static str>,
    has_unsplash_configured: bool,
    has_llm_configured: bool,
    file_size_limit: f64,
    is_smtp_configured: bool,
    admin_base_url: String,
    space_base_url: String,
    app_base_url: String,
    instance_changelog_url: &'static str,
    is_self_managed: bool,
}

#[derive(Serialize)]
struct Instance {
    id: &'static str,
    created_at: String,
    updated_at: String,
    deleted_at: Option<String>,
    instance_name: &'static str,
    whitelist_emails: Option<String>,
    instance_id: &'static str,
    current_version: &'static str,
    latest_version: &'static str,
    edition: &'static str,
    domain: &'static str,
    last_checked_at: String,
    namespace: Option<String>,
    is_telemetry_enabled: bool,
    is_support_required: bool,
    is_setup_done: bool,
    is_signup_screen_visited: bool,
    is_verified: bool,
    is_test: bool,
    is_current_version_deprecated: bool,
    created_by: Option<String>,
    updated_by: Option<String>,
    /// True once any workspace exists.
    workspaces_exist: bool,
}

/// The origin the browser used, for the three `*_base_url` values: Thrust serves the app and the API from one origin,
/// where Plane has a separate `WEB_URL`. `x-forwarded-proto` is trusted only for the scheme.
fn origin(headers: &HeaderMap) -> String {
    let host = headers.get(HOST).and_then(|v| v.to_str().ok()).unwrap_or("localhost");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|s| matches!(*s, "http" | "https"))
        .unwrap_or("http");
    format!("{scheme}://{host}")
}

/// S01. Always `is_setup_done: true`: there is no instance-admin setup step to wait for (research/0003 section 9,
/// row 3), and a `false` would send the visitor to Plane's god-mode setup.
pub(crate) async fn get_instance(Extension(app): Extension<App>, headers: HeaderMap) -> Result<Json<InstanceResponse>, PlaneError> {
    let now = app.ports.clock.now().to_rfc3339_opts(SecondsFormat::Micros, true);
    let origin = origin(&headers);
    let workspaces_exist = thrust_core::any_workspace_exists(&app).await?;
    Ok(Json(InstanceResponse {
        config: Config {
            enable_signup: true,
            is_workspace_creation_disabled: false,
            is_google_enabled: false,
            is_github_enabled: false,
            is_gitlab_enabled: false,
            is_gitea_enabled: false,
            is_oidc_enabled: false,
            is_magic_login_enabled: false,
            is_email_password_enabled: true,
            github_app_name: "",
            slack_client_id: None,
            has_unsplash_configured: false,
            has_llm_configured: false,
            file_size_limit: 5_242_880.0,
            is_smtp_configured: false,
            admin_base_url: origin.clone(),
            space_base_url: origin.clone(),
            app_base_url: origin,
            instance_changelog_url: CHANGELOG_URL,
            is_self_managed: true,
        },
        instance: Instance {
            id: INSTANCE_ROW_ID,
            created_at: now.clone(),
            updated_at: now.clone(),
            deleted_at: None,
            instance_name: INSTANCE_NAME,
            whitelist_emails: None,
            instance_id: INSTANCE_ID,
            current_version: env!("CARGO_PKG_VERSION"),
            latest_version: env!("CARGO_PKG_VERSION"),
            edition: "PLANE_COMMUNITY",
            domain: "",
            last_checked_at: now,
            namespace: None,
            is_telemetry_enabled: false,
            is_support_required: true,
            is_setup_done: true,
            is_signup_screen_visited: false,
            is_verified: false,
            is_test: false,
            is_current_version_deprecated: false,
            created_by: None,
            updated_by: None,
            workspaces_exist,
        },
    }))
}
