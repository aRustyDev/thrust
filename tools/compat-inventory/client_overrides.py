# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Hand-curated inputs for client_calls.py. Everything here is a human judgement, not a computed value.

If a key below no longer matches a call site after a re-run on a new Plane commit, client_calls.py's
stats show it under `override_keys_unused` (see run_all.py) - re-check it by reading the source.
"""

# Expressions that are the *origin* (scheme://host) of the API, not part of the path.
BASE_EXPRS = {
    "API_BASE_URL",
    "API_URL",
    "baseUrl",
    "baseURL",
    "this.baseURL",
    "process.env.VITE_API_BASE_URL",
    "import.meta.env.VITE_API_BASE_URL",
    "process.env.NEXT_PUBLIC_API_BASE_URL",
}

# `${this.<field>}` template expressions whose values are enum members (expanded from packages/types).
THIS_ENUM_ALIASES = {
    "this.serviceType": "EIssueServiceType",
}

# (file relative to the Plane tree root, placeholder label) -> replacement text, for abstract base classes whose
# path prefix is set by a subclass. apps/live PageCoreService builds `${this.basePath}/pages/...`; the only
# concrete subclass (ProjectPageService, apps/live/src/services/page/project-page.service.ts:29) sets
# basePath = `/api/workspaces/${workspaceSlug}/projects/${projectId}`.
PLACEHOLDER_SUBST = {
    ("apps/live/src/services/page/core.service.ts", "basePath"): "/api/workspaces/{workspaceSlug}/projects/{projectId}",
}

# (file relative to the Plane tree root, "Class.member", http verb) -> hand-resolved paths.
# `external: True` means the URL is not a Plane API path (presigned S3 URL, third-party API) and is
# excluded from the server join.
OVERRIDES = {
    # analytics: URL is assembled by this.processUrl() (apps/web/core/services/analytics.service.ts:98-111):
    #   `/api/workspaces/${slug}` [+ `/projects/${projectId}` when peek view of work items] + `/${endpoint}`
    ("apps/web/core/services/analytics.service.ts", "AnalyticsService.getAdvanceAnalytics", "get"): {
        "paths": [
            "/api/workspaces/{workspaceSlug}/advance-analytics",
            "/api/workspaces/{workspaceSlug}/projects/{projectId}/advance-analytics",
        ],
        "why": "hand-resolved from AnalyticsService.processUrl (endpoint literal 'advance-analytics')",
    },
    ("apps/web/core/services/analytics.service.ts", "AnalyticsService.getAdvanceAnalyticsStats", "get"): {
        "paths": [
            "/api/workspaces/{workspaceSlug}/advance-analytics-stats",
            "/api/workspaces/{workspaceSlug}/projects/{projectId}/advance-analytics-stats",
        ],
        "why": "hand-resolved from AnalyticsService.processUrl (endpoint literal 'advance-analytics-stats')",
    },
    ("apps/web/core/services/analytics.service.ts", "AnalyticsService.getAdvanceAnalyticsCharts", "get"): {
        "paths": [
            "/api/workspaces/{workspaceSlug}/advance-analytics-charts",
            "/api/workspaces/{workspaceSlug}/projects/{projectId}/advance-analytics-charts",
        ],
        "why": "hand-resolved from AnalyticsService.processUrl (endpoint literal 'advance-analytics-charts')",
    },
    # file upload: `url` is the presigned POST/PUT target returned by the asset endpoints (S3/MinIO), not a Plane path
    ("apps/web/core/services/file-upload.service.ts", "FileUploadService.uploadFile", "put"): {
        "paths": [], "external": True, "why": "presigned upload URL returned by /api/assets/v2/... (object store, not the API)",
    },
    ("apps/web/core/services/file-upload.service.ts", "FileUploadService.uploadFile", "post"): {
        "paths": [], "external": True, "why": "presigned upload URL returned by /api/assets/v2/... (object store, not the API)",
    },
    ("packages/services/src/file/file-upload.service.ts", "FileUploadService.uploadFile", "put"): {
        "paths": [], "external": True, "why": "presigned upload URL returned by /api/assets/v2/... (object store, not the API)",
    },
    ("packages/services/src/file/file-upload.service.ts", "FileUploadService.uploadFile", "post"): {
        "paths": [], "external": True, "why": "presigned upload URL returned by /api/assets/v2/... (object store, not the API)",
    },
    # deleteNewAsset(assetPath): callers pass an absolute asset URL built by getEditorAssetSrc()
    # (packages/utils/src/editor/common.ts:19-27 -> /api/assets/v2/workspaces/{slug}[/projects/{id}]/{assetId}/;
    #  apps/space/helpers/editor.helper.ts:19-22 -> /api/public/assets/v2/anchor/{anchor}/{assetId}/)
    ("apps/web/core/services/file.service.ts", "FileService.deleteNewAsset", "delete"): {
        "paths": [
            "/api/assets/v2/workspaces/{workspaceSlug}/projects/{projectId}/{assetId}",
            "/api/assets/v2/workspaces/{workspaceSlug}/{assetId}",
        ],
        "why": "hand-resolved from caller apps/web/core/hooks/editor/use-editor-config.ts:50 via getEditorAssetSrc",
    },
    ("packages/services/src/file/file.service.ts", "FileService.deleteNewAsset", "delete"): {
        "paths": [
            "/api/assets/v2/workspaces/{workspaceSlug}/projects/{projectId}/{assetId}",
            "/api/assets/v2/workspaces/{workspaceSlug}/{assetId}",
            "/api/public/assets/v2/anchor/{anchor}/{assetId}",
        ],
        "why": "hand-resolved from callers use-editor-config.ts:50 (web) and apps/space/helpers/editor.helper.ts:56 (space)",
    },
    # integrations: GitHub repositories list; url comes from the integration component (third-party API)
    ("apps/web/core/services/project/project.service.ts", "ProjectService.getGithubRepositories", "request"): {
        "paths": [], "external": True, "why": "url param is a GitHub API URL built in apps/web/core/components/integration/github/select-repository.tsx",
    },
}
