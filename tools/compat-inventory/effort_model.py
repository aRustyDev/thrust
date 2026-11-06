# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Effort classes and the person-day model for the compat-plane adapter.

CLASSIFICATION is computed from the route table (path shape + handler facts from the AST).
The UNIT COSTS below are judgement `[inferred]`: they are inputs, not measurements. Change a number and
join_tag.py recomputes every total. Scope of a "person-day": the *compat HTTP adapter only* (routing, authn
extraction, request parsing, response/serializer shaping, Plane error bodies, oracle-diff fixes) in Rust on
top of core use-cases that exist anyway for F-B01..F-B16. It excludes the core use-cases, the DB schema and
the identity subsystem themselves.
"""

import re

CLASSES = ["crud", "grouped-listing", "paginated-listing", "bulk", "file-upload", "auth-redirect", "aggregate-other"]

# Marginal person-days per HANDLER ROW (one HTTP method on one path), by complexity bucket.
# Buckets come from handler facts: S = loc<=20 and orm_calls<=3; M = loc<=60 and orm_calls<=12; L otherwise.
# Handlers inherited from DRF defaults (no project code) are S.
PER_ROW = {
    "crud":              {"S": 0.15, "M": 0.40, "L": 0.90},
    "paginated-listing": {"S": 0.50, "M": 0.80, "L": 1.20},
    "grouped-listing":   {"S": 1.00, "M": 1.50, "L": 2.50},   # marginal AFTER the group_by engine exists (FOUNDATIONS)
    "bulk":              {"S": 0.40, "M": 0.80, "L": 1.20},
    "file-upload":       {"S": 0.40, "M": 0.80, "L": 1.20},   # marginal AFTER the presigned-asset flow exists
    "auth-redirect":     {"S": 0.30, "M": 0.80, "L": 1.50},   # marginal AFTER csrf+session-form-flow exists (FOUNDATIONS)
    "aggregate-other":   {"S": 0.40, "M": 1.00, "L": 2.50},
}
# Optimistic / pessimistic multipliers applied to the marginal total of each class.
RANGE = {c: (0.7, 1.6) for c in CLASSES}
RANGE["grouped-listing"] = (0.7, 2.0)
RANGE["aggregate-other"] = (0.6, 2.0)

# One-off machinery shared by many rows: (person-days, what it is, where it lives in Plane).
FOUNDATIONS = {
    "route-scaffold+authn": (3.0, "axum router mounted under /api, /auth, /api/public, /api/instances; session cookie extraction -> permission seam; role decorator equivalent of @allow_permission (ROLE 5/15/20)", "app/permissions/base.py, authentication/session.py"),
    "error-bodies": (1.0, "three error shapes: DRF {detail}, project {error}, serializer {field:[msgs]}; map core errors onto them (status codes included)", "app/views/base.py handle_exception, authentication/adapter/exception.py (anchors in shapes-anchors.json)"),
    "pagination-envelope": (2.0, "cursor 'value:offset:is_prev', per_page cap 1000, envelope {grouped_by, sub_grouped_by, total_count, next_cursor, prev_cursor, next_page_results, prev_page_results, count, total_pages, total_results, extra_stats, results}", "utils/paginator.py BasePaginator.paginate"),
    "group-by-engine": (6.0, "group_by / sub_group_by listing incl. m2m FIELD_MAPPER (labels/assignees/modules), window-function paging per group, per-group total_results, group value enumeration", "utils/paginator.py GroupedOffsetPaginator/SubGroupedOffsetPaginator, utils/grouper.py"),
    "issue-filter-parser": (6.0, "work item filter machinery shared by every issue listing: legacy query params (25 names, issue_filters.py) AND the rich JSON `filters` tree (ComplexFilterBackend, max depth 5), order_by, sub_issue, expand, fields", "utils/issue_filters.py (463 lines), utils/filters/* , utils/order_queryset.py"),
    "expand-fields-projection": (2.0, "?expand= / ?fields= serializer projection (BaseViewSet.fields/expand)", "app/views/base.py fields/expand properties"),
    "csrf+session-form-flow": (4.0, "GET /auth/get-csrf-token/, form-POST sign-in/sign-up/sign-out with 302 + ?error_code=...&error_message=... + next_path validation, rate limits, session cookie 'session-id'", "authentication/views/app/*, authentication/adapter/error.py, authentication/rate_limit.py"),
    "presigned-asset-flow": (3.0, "POST create asset -> presigned POST/PUT (object store) -> PATCH confirm; GET -> 302 redirect to presigned URL; bulk/restore/duplicate", "app/views/asset/v2.py"),
    "serializer-shape-fidelity": (3.0, "oracle-diff iteration on the ~25 hot serializers (IssueSerializer incl. computed counts, ProjectSerializer, UserMeSerializer, WorkspaceSerializer ...): field names, nesting, null vs absent, date formats", "app/serializers/*"),
}

_BULK_RX = re.compile(r"/(bulk-[a-z-]+|issue-dates|transfer-issues|mark-all-read|duplicate-assets)/|/\{\}/bulk/$")


def bucket(row: dict) -> str:
    """Handler complexity from AST facts. S: <=25 lines and <=4 ORM calls; M: <=70 lines; L: bigger."""
    loc, orm = row.get("handler_loc_n"), row.get("orm_calls")
    if row.get("drf_default") or loc in (None, ""):
        return "S"
    loc, orm = int(loc), int(orm or 0)
    if loc <= 25 and orm <= 4:
        return "S"
    if loc <= 70:
        return "M"
    return "L"


_AGG_PATH_RX = re.compile(r"analytic|advance-analytics|search|export|dashboard|project-stats|progress|activity-graph|issues-completed-graph|"
                          r"user-stats|user-profile|user-activity|user-issues|/history/|/versions/|description-versions|recent-visits|unsplash|ai-assistant")


def classify(row: dict) -> str:
    """Effort class of one (method, path) row. Order matters; first match wins. All inputs are AST/path facts."""
    surface, method, path = row["surface"], row["method"], row["norm_path"]
    if surface == "auth":
        return "auth-redirect"
    if re.search(r"/assets/|/file-assets/|/issue-attachments/|/attachments/", path):
        return "file-upload"
    if _BULK_RX.search(path) or (re.search(r"/(cycle-issues|issues)/$", path) and method == "POST" and "cycles/{}" in path) \
            or (path.endswith("/modules/{}/issues/") and method == "POST") or path.endswith("/issues/{}/modules/"):
        return "bulk"
    if method == "GET" and row.get("mentions_group_by") in (True, "True"):
        return "grouped-listing"
    if method == "GET" and row.get("paginates") in (True, "True"):
        return "paginated-listing"
    agg = int(row.get("agg_calls") or 0)
    if agg >= 3 or _AGG_PATH_RX.search(path):
        return "aggregate-other"
    return "crud"


def row_days(row: dict) -> float:
    return PER_ROW[classify(row)][bucket(row)]
