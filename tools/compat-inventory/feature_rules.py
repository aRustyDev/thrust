# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Hand-curated tagging rules: server route -> FEATURES.md feature ids.

This file is judgement, not computation. Each rule is `(regex, methods, features, tier_override, note)`:
  * `regex` is matched against  "<surface>:<positional norm path>"  e.g. "internal:/api/workspaces/{}/projects/".
  * `methods` is None (any) or a set of HTTP methods the rule applies to.
  * `features` are ids from FEATURES.md (`F-B03`, ...); tiers are read from FEATURES.md at run time, so a tier
    change there flows through without editing this file.
  * `tier_override` is used only where the *route* sits at a different tier than its feature row (e.g.
    invitations are P1 although F-A07 is P0). Always give a reason in `note`.
First matching rule wins. A route with no matching rule is reported as `unmapped` (a bug in this table).
"""

import re

# fmt: off
_RAW = [
    # ------------------------------------------------------------------ instance (license app)
    (r"^instance:/api/instances/$", {"GET"}, ["F-A10"], None, "instance info: read at boot by web, space and admin (InstanceWrapper)"),
    (r"^instance:/api/instances/admins/(me/|session/)$", None, ["F-A10"], None, "instance-admin session (apps/admin only)"),
    (r"^instance:/api/instances/", None, ["F-A10"], None, "instance admin / configuration (apps/admin retired, Q-0021.2)"),

    # ------------------------------------------------------------------ internal: user
    (r"^internal:/api/users/me/$", None, ["F-A09", "F-A03"], None, "current user (session bootstrap + profile edit + deactivate)"),
    (r"^internal:/api/users/session/$", None, ["F-A03"], None, "session probe"),
    (r"^internal:/api/users/me/accounts/", None, ["F-A09"], None, "linked OAuth accounts"),
    (r"^internal:/api/users/me/email/", None, ["F-A09"], None, "email change (verification code)"),
    (r"^internal:/api/users/me/instance-admin/$", None, ["F-A10"], None, "is-instance-admin flag"),
    (r"^internal:/api/users/me/notification-preferences/$", None, ["F-D01"], None, "notification preferences"),
    (r"^internal:/api/users/me/(onboard|tour-completed|profile|settings|activities)/$", None, ["F-A09"], None, "profile / onboarding / settings"),
    (r"^internal:/api/users/me/workspaces/$", None, ["F-B01"], None, "workspace list for the current user"),
    (r"^internal:/api/users/me/workspaces/invitations/$", None, ["F-A07"], "P1", "workspace invites for the current user (invites are P1, FEATURES F-A07)"),
    (r"^internal:/api/users/me/workspaces/\{\}/(activity-graph|issues-completed-graph)/$", None, ["F-A09"], None, "profile graphs"),
    (r"^internal:/api/users/me/workspaces/\{\}/dashboard/$", None, ["F-B18"], None, "home dashboard"),
    (r"^internal:/api/users/me/workspaces/\{\}/project-roles/$", None, ["F-A08"], None, "project roles map (permission bootstrap)"),
    (r"^internal:/api/users/me/workspaces/\{\}/projects/invitations/$", None, ["F-A08"], "P1", "project invites for the current user (invites P1)"),
    (r"^internal:/api/users/api-tokens/", None, ["F-A06"], None, "API tokens (personal access tokens)"),
    (r"^internal:/api/users/file-assets/", None, ["F-D04"], None, "legacy user file assets"),
    (r"^internal:/api/users/last-visited-workspace/$", None, ["F-B18"], None, "last visited workspace"),
    (r"^internal:/api/timezones/$", None, ["F-A09"], None, "timezone list"),
    (r"^internal:/api/unsplash/$", None, ["F-D04"], None, "third-party Unsplash proxy for cover images"),
    (r"^internal:/api/workspace-slug-check/$", None, ["F-B01"], None, "slug availability"),
    # ------------------------------------------------------------------ internal: assets
    (r"^internal:/api/assets/v2/workspaces/\{\}/projects/\{\}/issues/\{\}/attachments/", None, ["F-B06", "F-D04"], None, "issue attachments v2 (presigned flow)"),
    (r"^internal:/api/assets/v2/", None, ["F-D04"], None, "file assets v2 (presign / confirm / redirect-to-object-store / bulk / restore)"),
    (r"^internal:/api/workspaces/file-assets/", None, ["F-D04"], None, "legacy workspace file assets"),
    (r"^internal:/api/workspaces/\{\}/file-assets/$", None, ["F-D04"], None, "legacy workspace file assets"),
    # ------------------------------------------------------------------ internal: workspace
    (r"^internal:/api/workspaces/$", None, ["F-B01"], None, "workspaces list/create"),
    (r"^internal:/api/workspaces/\{\}/$", None, ["F-B01"], None, "workspace detail/update/delete"),
    (r"^internal:/api/workspaces/\{\}/(advance-analytics|advance-analytics-stats|advance-analytics-charts|analytics|default-analytics|saved-analytic-view/\{\}|project-stats)/$", None, ["F-D07"], None, "analytics (P1)"),
    (r"^internal:/api/workspaces/\{\}/analytic-view/", None, ["F-D07"], None, "saved analytic views (P1)"),
    (r"^internal:/api/workspaces/\{\}/export-analytics/$", None, ["F-D07", "F-D05"], None, "analytics export (P1)"),
    (r"^internal:/api/workspaces/\{\}/ai-assistant/$", None, ["F-X08"], None, "AI assistant (P2, AI Gateway)"),
    (r"^internal:/api/workspaces/\{\}/cycles/$", None, ["F-B10"], None, "workspace-wide cycles"),
    (r"^internal:/api/workspaces/\{\}/modules/$", None, ["F-B11"], None, "workspace-wide modules"),
    (r"^internal:/api/workspaces/\{\}/estimates/$", None, ["F-B13"], None, "workspace estimates"),
    (r"^internal:/api/workspaces/\{\}/labels/$", None, ["F-B05"], None, "workspace labels"),
    (r"^internal:/api/workspaces/\{\}/states/$", None, ["F-B04"], None, "workspace states"),
    (r"^internal:/api/workspaces/\{\}/(draft-issues|draft-to-issue)/", None, ["F-B15"], None, "draft work items"),
    (r"^internal:/api/workspaces/\{\}/stickies/", None, ["F-B15"], None, "stickies"),
    (r"^internal:/api/workspaces/\{\}/(entity-search|search)/$", None, ["F-D08"], None, "global / entity search (P0 adapter: DB search)"),
    (r"^internal:/api/workspaces/\{\}/export-issues/$", None, ["F-D05"], None, "export work items (P1)"),
    (r"^internal:/api/workspaces/\{\}/user-activity/\{\}/export/$", None, ["F-D05"], None, "export user activity (P1)"),
    (r"^internal:/api/workspaces/\{\}/(home-preferences|quick-links|recent-visits|sidebar-preferences|user-favorites|user-favorite-projects)/", None, ["F-B18"], None, "favorites / home / recents / sidebar"),
    (r"^internal:/api/workspaces/\{\}/invitations/", None, ["F-A07"], "P1", "workspace invitations (invites are P1)"),
    (r"^internal:/api/workspaces/\{\}/issues/$", None, ["F-B16", "F-B12"], None, "workspace-level work item listing (grouped)"),
    (r"^internal:/api/workspaces/\{\}/members/", None, ["F-A07"], None, "workspace members"),
    (r"^internal:/api/workspaces/\{\}/workspace-members/me/$", None, ["F-A07"], None, "my workspace membership/role (permission bootstrap)"),
    (r"^internal:/api/workspaces/\{\}/project-members/$", None, ["F-A08"], None, "all project members of the workspace"),
    (r"^internal:/api/workspaces/\{\}/project-identifiers/$", None, ["F-B02"], None, "project identifier availability"),
    (r"^internal:/api/workspaces/\{\}/projects/$", None, ["F-B02"], None, "projects list/create"),
    (r"^internal:/api/workspaces/\{\}/projects/details/$", None, ["F-B02"], None, "projects (detailed list)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/$", None, ["F-B02"], None, "project detail/update/delete"),
    (r"^internal:/api/workspaces/\{\}/(user-activity|user-issues|user-profile|user-stats)/\{\}/$", None, ["F-A09"], None, "user profile pages"),
    (r"^internal:/api/workspaces/\{\}/user-properties/$", None, ["F-A09"], None, "workspace user properties (display)"),
    (r"^internal:/api/workspaces/\{\}/user-favorites/", None, ["F-B18"], None, "favorites"),
    (r"^internal:/api/workspaces/\{\}/users/notifications/", None, ["F-D01"], None, "in-app notifications"),
    (r"^internal:/api/workspaces/\{\}/views/", None, ["F-B12"], None, "workspace views"),
    (r"^internal:/api/workspaces/\{\}/workspace-views/$", None, ["F-B12"], None, "workspace views (user views)"),
    (r"^internal:/api/workspaces/\{\}/(webhooks|webhook-logs)/", None, ["F-D03"], None, "webhooks"),
    (r"^internal:/api/workspaces/\{\}/work-items/\{\}-\{\}/$", None, ["F-B03"], None, "work item by PROJ-123 identifier"),
    (r"^internal:/api/workspaces/\{\}/workspace-themes/", None, ["F-A09"], None, "workspace themes"),
    # ------------------------------------------------------------------ internal: project
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/ai-assistant/$", None, ["F-X08"], None, "AI assistant (P2)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/archive/$", None, ["F-B02"], None, "archive project"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/archived-cycles/", None, ["F-B10"], None, "archived cycles"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/archived-modules/", None, ["F-B11"], None, "archived modules"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/archived-issues/$", None, ["F-B03", "F-B16"], None, "archived work items (listing)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/deleted-issues/$", None, ["F-B03", "F-B16"], None, "deleted work items (listing)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/bulk-create-labels/$", None, ["F-B05", "F-B16"], None, "bulk label creation"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/(bulk-archive-issues|bulk-delete-issues|issue-dates)/$", None, ["F-B16"], None, "bulk operations"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/comments/\{\}/reactions/", None, ["F-B07"], None, "comment reactions"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/cycles/\{\}/cycle-issues/", None, ["F-B10", "F-B16"], None, "cycle membership of work items (grouped listing / add)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/cycles/\{\}/user-properties/$", None, ["F-B12"], None, "cycle display properties"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/cycles/", None, ["F-B10"], None, "cycles"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/(estimates|project-estimates)/", None, ["F-B13"], None, "estimates"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/favorite-pages/", None, ["F-C01"], None, "page favorites"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/intake-work-items/", None, ["F-B14", "F-B09"], None, "intake work item description versions"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/(inbox-issues|inboxes|intake-issues|intakes|intake-state)/", None, ["F-B14"], None, "intake (triage)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/invitations/", None, ["F-A08"], "P1", "project invitations (invites P1)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/join/\{\}/$", None, ["F-A08"], "P1", "accept project invitation"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issue-labels/", None, ["F-B05"], None, "project labels"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/(issues-detail|issues/list|v2/issues)/$", None, ["F-B03", "F-B16"], None, "work item listings (envelope / sync)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/$", None, ["F-B03", "F-B16"], None, "work item list (grouped, paginated) / create"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/\{\}/$", None, ["F-B03"], None, "work item detail/update/delete"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/\{\}/archive/$", None, ["F-B03"], None, "archive work item"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/\{\}/comments/", None, ["F-B07"], None, "work item comments"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/\{\}/history/$", None, ["F-B08"], None, "activity history"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/\{\}/issue-attachments/", None, ["F-B06", "F-D04"], None, "legacy issue attachments"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/\{\}/issue-links/", None, ["F-B06"], None, "work item links"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/\{\}/(issue-relation|remove-relation)/", None, ["F-B06"], None, "work item relations"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/\{\}/(issue-subscribers|subscribe)/", None, ["F-B05"], None, "subscribers"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/\{\}/meta/$", None, ["F-B16"], None, "work item meta"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/\{\}/modules/$", None, ["F-B11"], None, "add work item to modules"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/\{\}/reactions/", None, ["F-B07"], None, "work item reactions"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/\{\}/sub-issues/$", None, ["F-B03"], None, "sub-work-items"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/issues/\{\}/versions/", None, ["F-B09"], None, "work item versions"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/work-items/\{\}/description-versions/", None, ["F-B09"], None, "description versions"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/(members|project-members/me|preferences/member/\{\})/", None, ["F-A08"], None, "project members / my project role / member preferences"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/modules/\{\}/user-properties/$", None, ["F-B12"], None, "module display properties"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/modules/\{\}/issues/", None, ["F-B11", "F-B16"], None, "module membership of work items (grouped listing / add)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/modules/", None, ["F-B11"], None, "modules (+links, archive)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/pages/\{\}/versions/", None, ["F-C03"], None, "page versions (P1)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/(pages|pages-summary)/", None, ["F-C01"], None, "pages"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/project-deploy-boards/", None, ["F-D09"], None, "publish project (deploy board)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/project-views/$", None, ["F-B12"], None, "project views (user tracking)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/search-issues/$", None, ["F-D08"], None, "issue search (pickers)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/states/", None, ["F-B04"], None, "states"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/user-favorite-(cycles|modules|views)/", None, ["F-B18"], None, "favorites (cycles/modules/views)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/user-properties/$", None, ["F-B12"], None, "project display properties (per user)"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/views/", None, ["F-B12"], None, "project views"),
    (r"^internal:/api/workspaces/\{\}/projects/\{\}/advance-analytics", None, ["F-D07"], None, "project analytics (P1)"),
    # ------------------------------------------------------------------ space (published boards)
    (r"^space:/api/public/anchor/\{\}/intakes/", None, ["F-D09", "F-B14"], None, "public intake submission"),
    (r"^space:/api/public/anchor/\{\}/issues/\{\}/(comments|reactions|votes)/", None, ["F-D09", "F-B07"], None, "public comments / reactions / votes"),
    (r"^space:/api/public/anchor/\{\}/comments/\{\}/reactions/", None, ["F-D09", "F-B07"], None, "public comment reactions"),
    (r"^space:/api/public/assets/", None, ["F-D09", "F-D04"], None, "public assets"),
    (r"^space:/api/public/", None, ["F-D09"], None, "published boards"),
    # ------------------------------------------------------------------ auth (server-redirect flows)
    (r"^auth:/auth/(spaces/)?(sign-in|sign-up|sign-out|get-csrf-token|email-check|forgot-password|reset-password/\{\}/\{\}|change-password|set-password)/$", None, ["F-A03"], None, "email+password auth / csrf / reset"),
    (r"^auth:/auth/(spaces/)?magic-(generate|sign-in|sign-up)/$", None, ["F-A04"], None, "magic code (email OTP)"),
    (r"^auth:/auth/(spaces/)?(google|github|gitlab|gitea)/(callback/)?$", None, ["F-A05"], None, "OAuth providers"),
    (r"^auth:/auth/(spaces/)?oidc/(callback/)?$", None, ["F-A01"], None, "OIDC SSO (woven addition)"),
    # ------------------------------------------------------------------ v1 (public REST API; ADR-0031) and others
    (r"^v1:", None, ["F-A5"], None, "public REST API v1 (wire-compatible; NOT part of compat-plane)"),
    (r"^web:", None, ["F-X07"], None, "health / robots"),
    (r"^schema:", None, ["F-X13"], "drop", "drf-spectacular schema UI (off unless ENABLE_DRF_SPECTACULAR)"),
]
# fmt: on

RULES = [(re.compile(rx), ms, feats, tier, note) for rx, ms, feats, tier, note in _RAW]


def tag(surface: str, norm_path: str, method: str):
    key = f"{surface}:{norm_path}"
    for i, (rx, ms, feats, tier, note) in enumerate(RULES):
        if rx.search(key) and (ms is None or method in ms):
            return {"rule": i, "features": feats, "tier_override": tier, "note": note}
    return None
