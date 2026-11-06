# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""The minimum P0 slice, hand-traced from the React app and VALIDATED by effort_report.py.

Journey: first user -> sign up -> onboarding -> create workspace -> workspace home -> create project -> open project
-> create work item -> see it in the project list, through the UNMODIFIED React app.

Each entry names the server route (method + positional path, as in server-routes.json), the client call that fires it
(`<client>:<Class>.<member>` as in client-calls.json, or `ref:<kind>` for a form post / redirect / asset URL), and the
file:line that triggers it. effort_report.py fails the run if a route does not exist in the server table or the named
client call is not found: after a re-run on a new Plane commit, a stale entry is therefore loud, not silent.

`stage` values: boot (every page load) | auth | onboarding | workspace-load (every workspace page) | home |
project-create | project-open (every project page) | work-item.
`kind`: min = needed for the journey above; ext = needed to *view/edit* the created work item (peek) - not minimum.
All trigger lines were read on commit recorded in out/RUN.json.
"""

SLICE = [
    # ---- boot (every page load, any app) -------------------------------------------------------------------------
    dict(id="S01", stage="boot", kind="min", method="GET", path="/api/instances/", call="web:InstanceService.getInstanceInfo",
         trigger="apps/web/core/lib/wrappers/instance-wrapper.tsx:24",
         why="InstanceWrapper blocks the whole app: web's getInstanceInfo has NO validateStatus override, so non-2xx -> SWR error -> MaintenanceView; `is_setup_done:false` renders InstanceNotReady and stops (space/admin use @plane/services InstanceService.info with validateStatus:null)"),
    dict(id="S02", stage="boot", kind="min", method="GET", path="/api/users/me/", call="web:UserService.currentUser",
         trigger="apps/web/core/lib/wrappers/authentication-wrapper.tsx:41",
         why="AuthenticationWrapper; `validateStatus: null` so the 401 JSON body {detail} (no `id`) means 'signed out' without tripping the 401 interceptor; with `id` it fans out to S07-S09"),
    # ---- auth (browser form posts, 302 flows) ---------------------------------------------------------------------
    dict(id="S03", stage="auth", kind="min", method="GET", path="/auth/get-csrf-token/", call="web:AuthService.requestCSRFToken",
         trigger="apps/web/core/components/account/auth-forms/password.tsx:74",
         why="CSRF cookie is HttpOnly, so the SPA fetches the token as JSON and copies it into a hidden `csrfmiddlewaretoken` form field"),
    dict(id="S04", stage="auth", kind="min", method="POST", path="/auth/email-check/", call="web:AuthService.emailCheck",
         trigger="apps/web/core/components/account/auth-forms/form-root.tsx:57", why="decides sign-in vs sign-up UI (JSON POST via axios, not a form)"),
    dict(id="S05", stage="auth", kind="min", method="POST", path="/auth/sign-up/", call="ref:form-action",
         trigger="apps/web/core/components/account/auth-forms/password.tsx:151", why="HTML form POST -> always 302 (to next_path or ?error_code=...)"),
    dict(id="S06", stage="auth", kind="min", method="POST", path="/auth/sign-in/", call="ref:form-action",
         trigger="apps/web/core/components/account/auth-forms/password.tsx:151", why="same form, mode switch"),
    # ---- after login: Promise.all in UserStore.fetchCurrentUser -------------------------------------------------
    dict(id="S07", stage="boot", kind="min", method="GET", path="/api/users/me/profile/", call="web:UserService.getCurrentUserProfile",
         trigger="apps/web/core/store/user/index.ts:121", why="Promise.all after /users/me/ (is_onboarded drives routing)"),
    dict(id="S08", stage="boot", kind="min", method="GET", path="/api/users/me/settings/", call="web:UserService.currentUserSettings",
         trigger="apps/web/core/store/user/index.ts:122", why="last/fallback workspace slug for the post-login redirect"),
    dict(id="S09", stage="boot", kind="min", method="GET", path="/api/users/me/workspaces/", call="web:WorkspaceService.userWorkspaces",
         trigger="apps/web/core/store/user/index.ts:123", why="workspace list (also re-fetched after create-workspace)"),
    # ---- onboarding ----------------------------------------------------------------------------------------------
    dict(id="S10", stage="onboarding", kind="min", method="PATCH", path="/api/users/me/", call="web:UserService.updateUser",
         trigger="apps/web/core/components/onboarding/steps/profile/root.tsx:96", why="first/last name, avatar"),
    dict(id="S11", stage="onboarding", kind="min", method="PATCH", path="/api/users/me/profile/", call="web:UserService.updateCurrentUserProfile",
         trigger="apps/web/core/components/onboarding/steps/workspace/create.tsx:109", why="onboarding_step flags, is_onboarded, role, use_case"),
    dict(id="S12", stage="onboarding", kind="min", method="GET", path="/api/users/me/workspaces/invitations/", call="web:WorkspaceService.userWorkspaceInvitations",
         trigger="apps/web/app/(all)/onboarding/page.tsx:43", why="fires on entering onboarding; may return [] (invites are P1, but the route must exist)"),
    dict(id="S13", stage="onboarding", kind="min", method="GET", path="/api/workspace-slug-check/", call="web:WorkspaceService.workspaceSlugCheck",
         trigger="apps/web/core/components/onboarding/steps/workspace/create.tsx:75", why="`{status: boolean}` shape"),
    dict(id="S14", stage="onboarding", kind="min", method="POST", path="/api/workspaces/", call="web:WorkspaceService.createWorkspace",
         trigger="apps/web/core/components/onboarding/steps/workspace/create.tsx:79", why="creator becomes workspace admin (ROLE 20)"),
    # ---- workspace-load: WorkspaceAuthWrapper (every /{workspaceSlug}/* page) ------------------------------------
    dict(id="S15", stage="workspace-load", kind="min", method="GET", path="/api/workspaces/{}/workspace-members/me/", call="web:WorkspaceService.workspaceMemberMe",
         trigger="apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:76", why="my workspace role: drives every `allowPermissions` check in the UI"),
    dict(id="S16", stage="workspace-load", kind="min", method="GET", path="/api/users/me/workspaces/{}/project-roles/", call="web:WorkspaceService.getWorkspaceUserProjectsRole",
         trigger="apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:81", why="map project_id -> my role (permission bootstrap)"),
    dict(id="S17", stage="workspace-load", kind="min", method="GET", path="/api/workspaces/{}/projects/", call="web:ProjectService.getProjectsLite",
         trigger="apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:88", why="partial project list for the sidebar"),
    dict(id="S18", stage="workspace-load", kind="min", method="GET", path="/api/workspaces/{}/members/", call="web:WorkspaceService.fetchWorkspaceMembers",
         trigger="apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:94", why="member directory (assignee pickers)"),
    dict(id="S19", stage="workspace-load", kind="min", method="GET", path="/api/workspaces/{}/user-favorites/", call="web:FavoriteService.getFavorites",
         trigger="apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:100", why="only when role >= member; sidebar favorites"),
    dict(id="S20", stage="workspace-load", kind="min", method="GET", path="/api/workspaces/{}/states/", call="web:ProjectStateService.getWorkspaceStates",
         trigger="apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:110", why="all states in the workspace"),
    dict(id="S21", stage="workspace-load", kind="min", method="GET", path="/api/workspaces/{}/sidebar-preferences/", call="web:WorkspaceService.fetchSidebarNavigationPreferences",
         trigger="apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:117", why="sidebar nav prefs; server creates defaults on first read [inferred]"),
    dict(id="S22", stage="workspace-load", kind="min", method="GET", path="/api/workspaces/{}/user-properties/", call="web:WorkspaceService.fetchWorkspaceFilters",
         trigger="apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:124", why="workspace-level display properties/filters"),
    dict(id="S23", stage="workspace-load", kind="min", method="GET", path="/api/workspaces/{}/users/notifications/unread/", call="web:WorkspaceNotificationService.fetchUnreadNotificationsCount",
         trigger="apps/web/core/components/workspace-notifications/notification-app-sidebar-option.tsx:29", why="sidebar badge; fires on every workspace page"),
    # ---- home ----------------------------------------------------------------------------------------------------
    dict(id="S24", stage="home", kind="min", method="GET", path="/api/workspaces/{}/home-preferences/", call="web:WorkspaceService.fetchWorkspaceWidgets",
         trigger="apps/web/core/components/home/root.tsx:29", why="home widgets list"),
    dict(id="S25", stage="home", kind="min", method="GET", path="/api/workspaces/{}/quick-links/", call="web:WorkspaceService.fetchWorkspaceLinks",
         trigger="apps/web/core/components/home/widgets/links/root.tsx:32", why="quick-links widget"),
    dict(id="S26", stage="home", kind="min", method="GET", path="/api/workspaces/{}/recent-visits/", call="web:WorkspaceService.fetchWorkspaceRecents",
         trigger="apps/web/core/components/home/widgets/recents/index.tsx:48", why="recents widget"),
    dict(id="S27", stage="home", kind="min", method="PATCH", path="/api/users/me/tour-completed/", call="web:UserService.updateUserTourCompleted",
         trigger="apps/web/core/components/home/root.tsx:36", why="first visit shows a full-screen tour until `is_tour_completed`; skipping it PATCHes this"),
    # ---- project create (stock UI uploads a default cover image: presigned-asset flow is IN the slice) -----------
    dict(id="S28", stage="project-create", kind="min", method="POST", path="/api/assets/v2/workspaces/{}/", call="web:FileService.uploadWorkspaceAsset",
         trigger="apps/web/ce/components/projects/create/root.tsx:74", why="getProjectFormValues() preselects a random STATIC cover (`local_static`) -> uploaded to the object store on submit"),
    dict(id="S29", stage="project-create", kind="min", method="PATCH", path="/api/assets/v2/workspaces/{}/{}/", call="web:FileService.updateWorkspaceAssetUploadStatus",
         trigger="apps/web/core/services/file.service.ts:92", why="confirm upload (after the browser POSTs to the presigned URL)"),
    dict(id="S30", stage="project-create", kind="min", method="POST", path="/api/workspaces/{}/projects/", call="web:ProjectService.createProject",
         trigger="apps/web/ce/components/projects/create/root.tsx:96", why="creates project + default states + creator membership"),
    dict(id="S31", stage="project-create", kind="min", method="POST", path="/api/assets/v2/workspaces/{}/projects/{}/{}/bulk/", call="web:FileService.updateBulkProjectAssetsUploadStatus",
         trigger="apps/web/core/components/project/create-project-modal.tsx:57", why="attach the uploaded cover to the new project"),
    dict(id="S32", stage="project-create", kind="min", method="PATCH", path="/api/workspaces/{}/projects/{}/", call="web:ProjectService.updateProject",
         trigger="apps/web/ce/components/projects/create/root.tsx:100", why="set cover_image_url"),
    dict(id="S33", stage="project-create", kind="min", method="GET", path="/api/assets/v2/workspaces/{}/{}/", call="ref:asset-url",
         trigger="packages/utils/src/editor/common.ts:26", why="<img src> for uploaded assets -> 302 to a presigned GET [inferred: the cover uses the same route via getFileURL(asset_url)]"),
    # ---- project-open: ProjectAuthWrapper (every /{ws}/projects/{id}/* page) -------------------------------------
    dict(id="S34", stage="project-open", kind="min", method="GET", path="/api/workspaces/{}/projects/{}/", call="web:ProjectService.getProject",
         trigger="apps/web/core/layouts/auth-layout/project-wrapper.tsx:83", why="project detail"),
    dict(id="S35", stage="project-open", kind="min", method="GET", path="/api/workspaces/{}/project-stats/", call="web:ProjectService.getProjectAnalyticsCount",
         trigger="apps/web/core/store/project/project.store.ts:388", why="second half of fetchProjectDetails"),
    dict(id="S36", stage="project-open", kind="min", method="GET", path="/api/workspaces/{}/projects/{}/project-members/me/", call="web:ProjectMemberService.projectMemberMe",
         trigger="apps/web/core/layouts/auth-layout/project-wrapper.tsx:88", why="my project role"),
    dict(id="S37", stage="project-open", kind="min", method="GET", path="/api/workspaces/{}/projects/{}/user-properties/", call="web:ProjectService.getProjectUserProperties",
         trigger="apps/web/core/layouts/auth-layout/project-wrapper.tsx:90", why="display_filters / display_properties / rich filters (default `group_by: null, layout: list`)"),
    dict(id="S38", stage="project-open", kind="min", method="GET", path="/api/workspaces/{}/projects/{}/issue-labels/", call="web:IssueLabelService.getProjectLabels",
         trigger="apps/web/core/layouts/auth-layout/project-wrapper.tsx:96", why="labels"),
    dict(id="S39", stage="project-open", kind="min", method="GET", path="/api/workspaces/{}/projects/{}/members/", call="web:ProjectMemberService.fetchProjectMembers",
         trigger="apps/web/core/layouts/auth-layout/project-wrapper.tsx:101", why="project members"),
    dict(id="S40", stage="project-open", kind="min", method="GET", path="/api/workspaces/{}/projects/{}/states/", call="web:ProjectStateService.getStates",
         trigger="apps/web/core/layouts/auth-layout/project-wrapper.tsx:106", why="states"),
    dict(id="S41", stage="project-open", kind="min", method="GET", path="/api/workspaces/{}/projects/{}/intake-state/", call="web:ProjectStateService.getIntakeState",
         trigger="apps/web/core/layouts/auth-layout/project-wrapper.tsx:111", why="triage state (fires even when intake is off)"),
    dict(id="S42", stage="project-open", kind="min", method="GET", path="/api/workspaces/{}/projects/{}/estimates/", call="web:EstimateService.fetchProjectEstimates",
         trigger="apps/web/core/layouts/auth-layout/project-wrapper.tsx:116", why="estimates"),
    dict(id="S43", stage="project-open", kind="min", method="GET", path="/api/workspaces/{}/projects/{}/cycles/", call="web:CycleService.getCyclesWithParams",
         trigger="apps/web/core/layouts/auth-layout/project-wrapper.tsx:121", why="cycles (`cycle_view` param)"),
    dict(id="S44", stage="project-open", kind="min", method="GET", path="/api/workspaces/{}/modules/", call="web:ModuleService.getWorkspaceModules",
         trigger="apps/web/core/layouts/auth-layout/project-wrapper.tsx:129", why="fetchModulesSlim"),
    dict(id="S45", stage="project-open", kind="min", method="GET", path="/api/workspaces/{}/projects/{}/modules/", call="web:ModuleService.getModules",
         trigger="apps/web/core/layouts/auth-layout/project-wrapper.tsx:129", why="fetchModules"),
    dict(id="S46", stage="project-open", kind="min", method="GET", path="/api/workspaces/{}/projects/{}/views/", call="web:ViewService.getViews",
         trigger="apps/web/core/layouts/auth-layout/project-wrapper.tsx:134", why="saved views"),
    # ---- work item -----------------------------------------------------------------------------------------------
    dict(id="S47", stage="work-item", kind="min", method="GET", path="/api/workspaces/{}/projects/{}/issues/", call="web:IssueService.getIssuesFromServer", scale=0.5,
         trigger="apps/web/core/store/issue/project/issue.store.ts:117", why="flat paginated list (default display filter group_by=null); GROUPED/SUB-GROUPED envelope only after the user picks Kanban or a group"),
    dict(id="S48", stage="work-item", kind="min", method="POST", path="/api/workspaces/{}/projects/{}/issues/", call="web:IssueService.createIssue",
         trigger="apps/web/core/store/issue/helpers/base-issues.store.ts:534", why="allocates sequence_id; response is stored as-is by the MobX store"),
    # ---- extension: view/edit the created work item (peek) ------------------------------------------------------
    dict(id="X01", stage="work-item", kind="ext", method="GET", path="/api/workspaces/{}/projects/{}/issues/{}/", call="web:IssueService.retrieve",
         trigger="apps/web/core/components/issues/peek-overview/root.tsx:220", why="peek overview"),
    dict(id="X02", stage="work-item", kind="ext", method="GET", path="/api/workspaces/{}/projects/{}/issues/{}/history/", call="web:IssueActivityService.getIssueActivities",
         trigger="apps/web/core/services/issue/issue_comment.service.ts:36", why="activity AND comments (comments are listed via /history/?activity_type=issue-comment, not /comments/)"),
    dict(id="X03", stage="work-item", kind="ext", method="PATCH", path="/api/workspaces/{}/projects/{}/issues/{}/", call="web:IssueService.patchIssue",
         trigger="apps/web/core/services/issue/issue.service.ts:227", why="field edits (PUT is never used)"),
]

# Foundations (effort_model.FOUNDATIONS keys) the slice needs, as (fraction of the full foundation, why).
# A foundation only partly needed by the slice is scaled by a stated fraction (judgement, `[inferred]`).
SLICE_FOUNDATIONS = {
    "route-scaffold+authn": (1.0, "full"),
    "error-bodies": (1.0, "full"),
    "pagination-envelope": (0.5, "flat OffsetPaginator envelope only; grouped / sub-grouped envelopes are follow-on"),
    "issue-filter-parser": (0.25, "order_by, sub_issue, type, layout, `filters={}` only; legacy filter set and rich filter trees are follow-on"),
    "csrf+session-form-flow": (0.75, "sign-up/sign-in/sign-out + csrf token + next_path; magic / OAuth / OIDC are follow-on"),
    "presigned-asset-flow": (0.6, "workspace asset create/confirm/302 + project bulk-attach; user assets, duplicate, restore are follow-on"),
    "serializer-shape-fidelity": (0.4, "~10 of ~25 hot serializers: User/Profile/Settings, Workspace, WorkspaceMember, Project, State, Label, Member, Issue list+create"),
}

# Evidence check for the `trigger` file:line of each entry (effort_report.check_triggers). The trigger paths above are written in
# the pre-pin layout (apps/web/core/...); layout.py resolves them to wherever the file lives in the tree being scanned. `token` is
# a string that must appear within +-5 lines of the cited line; if it appears elsewhere in the file the entry is reported as
# "line drift" with the new line numbers, and if it does not appear in the file at all the run fails (the evidence is stale).
TRIGGER_TOKENS = {
    "S01": "fetchInstanceInfo", "S02": "fetchCurrentUser", "S03": "requestCSRFToken", "S04": "emailCheck",
    "S05": "action=", "S06": "action=",
    "S07": "fetchUserProfile", "S08": "fetchCurrentUserSettings", "S09": "fetchWorkspaces",
    "S10": "updateCurrentUser", "S11": "updateUserProfile", "S12": "userWorkspaceInvitations",
    "S13": "workspaceSlugCheck", "S14": "createWorkspace",
    "S15": "fetchUserWorkspaceInfo", "S16": "fetchUserProjectPermissions", "S17": "fetchPartialProjects",
    "S18": "fetchWorkspaceMembers", "S19": "fetchFavorite", "S20": "fetchWorkspaceStates",
    "S21": "fetchSidebarNavigationPreferences", "S22": "fetchProjectNavigationPreferences",
    "S23": "getUnreadNotificationsCount", "S24": "fetchWidgets", "S25": "fetchLinks", "S26": "fetchWorkspaceRecents",
    "S27": "updateTourCompleted",
    "S28": "uploadCoverImage", "S29": "updateWorkspaceAssetUploadStatus", "S30": "createProject",
    "S31": "updateBulkProjectAssetsUploadStatus", "S32": "updateProject", "S33": "getFileURL",
    "S34": "fetchProjectDetails", "S35": "getProjectAnalyticsCount", "S36": "fetchUserProjectInfo",
    "S37": "fetchProjectUserProperties", "S38": "fetchProjectLabels", "S39": "fetchProjectMembers",
    "S40": "fetchProjectStates", "S41": "fetchProjectIntakeState", "S42": "getProjectEstimates", "S43": "fetchAllCycles",
    "S44": "fetchModulesSlim", "S45": "fetchModules", "S46": "fetchViews",
    "S47": "getIssues", "S48": "createIssue",
    "X01": "issueOperations.fetch", "X02": "history", "X03": "patchIssue",
}
