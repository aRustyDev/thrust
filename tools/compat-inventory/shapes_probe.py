# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Step 5 (shape catalogue, source facts) - extracts the facts the findings file cites, WITH file:line, from source.

Nothing here is hand-typed except the list of places to look. On a new Plane commit, an anchor that moved shows a new
line; an anchor that vanished is reported `found: false` (the catalogue entry needs re-reading).

Usage:  uv run shapes_probe.py --root <plane tree root> --out <file.json>
"""

from __future__ import annotations

import argparse
import ast
import json
import re
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import layout  # noqa: E402
from client_calls import resolve_string_union, walk_files  # noqa: E402

API = "apps/api/plane"
_L: "layout.Layout | None" = None  # set by run(); every path below goes through it (loud on a missing path)


def rd(root: Path, rel: str) -> str:
    return (root / _L.resolve(rel)).read_text(encoding="utf-8")


def grep(root: Path, rel: str, pattern: str, flags=0, expect: int = 1):
    """Matching lines of `rel` (resolved through the layout helper; loud if the file is missing).

    `expect=1` means at least one match is expected, `expect=0` means none; a violation is RECORDED in the layout report
    (anchor drift), not hidden. `loc` uses the path the file actually has in this tree."""
    used = _L.resolve(rel)
    text = (root / used).read_text(encoding="utf-8")
    out = []
    for i, line in enumerate(text.splitlines(), 1):
        if re.search(pattern, line, flags):
            out.append({"loc": f"{used}:{i}", "text": line.strip()[:160]})
    _L.anchor_checks.append({"file": used, "pattern": pattern, "matches": len(out), "expect": "none" if expect == 0 else "at least one",
                             "ok": (len(out) == 0) if expect == 0 else (len(out) > 0)})
    return out


def py_files(root: Path, rel: str):
    base = root / _L.resolve_dir(rel)
    files = sorted(p for p in base.rglob("*.py") if "migrations" not in p.parts)
    if not files:
        raise layout.LayoutError(f"no .py files under {rel}")
    return files


def dict_keys_in_response(fn: ast.FunctionDef):
    """Keys of the dict literal passed to `Response(...)` inside fn."""
    for n in ast.walk(fn):
        if isinstance(n, ast.Call) and getattr(n.func, "id", "") == "Response" and n.args and isinstance(n.args[0], ast.Dict):
            return [(k.value, k.lineno) for k in n.args[0].keys if isinstance(k, ast.Constant)]
    return []


def run(root: Path) -> dict:
    global _L
    _L = layout.for_root(root)
    out: dict = {}

    # ------------------------------------------------------------------ A. pagination
    pg = root / _L.resolve(f"{API}/utils/paginator.py")
    tree = ast.parse(pg.read_text())
    A: dict = {"file": f"{API}/utils/paginator.py"}
    for node in ast.walk(tree):
        if isinstance(node, ast.ClassDef) and node.name == "BasePaginator":
            for fn in node.body:
                if isinstance(fn, ast.FunctionDef) and fn.name == "paginate":
                    A["envelope_keys"] = [{"key": k, "line": ln} for k, ln in dict_keys_in_response(fn)]
                    defaults = {}
                    for a, d in zip(reversed(fn.args.args), reversed(fn.args.defaults)):
                        defaults[a.arg] = ast.unparse(d)
                    A["paginate_defaults"] = {k: defaults[k] for k in ("default_per_page", "max_per_page", "paginator_cls") if k in defaults}
                    A["paginate_loc"] = f"{API}/utils/paginator.py:{fn.lineno}"
        if isinstance(node, ast.ClassDef) and node.name == "Cursor":
            for fn in node.body:
                if isinstance(fn, ast.FunctionDef) and fn.name == "__str__":
                    A["cursor_str"] = ast.unparse(fn.body[-1])
                    A["cursor_str_loc"] = f"{API}/utils/paginator.py:{fn.lineno}"
        if isinstance(node, ast.ClassDef) and node.name in ("OffsetPaginator", "GroupedOffsetPaginator", "SubGroupedOffsetPaginator"):
            for fn in node.body:
                if isinstance(fn, ast.FunctionDef) and fn.name == "get_result":
                    A.setdefault("get_result_default_limit", {})[node.name] = {
                        "limit_default": ast.unparse(fn.args.defaults[0]) if fn.args.defaults else None, "loc": f"{API}/utils/paginator.py:{fn.lineno}"}
    A["MAX_LIMIT"] = grep(root, f"{API}/utils/paginator.py", r"^MAX_LIMIT")
    A["field_mapper_m2m"] = grep(root, f"{API}/utils/paginator.py", r'"(labels__id|assignees__id|issue_module__module_id)": "')[:3]
    A["row_number_window"] = grep(root, f"{API}/utils/paginator.py", r"row_number=Window")[:2]
    out["pagination"] = A

    # call sites of .paginate( across app views
    calls = []
    for f in py_files(root, f"{API}/app/views") + py_files(root, f"{API}/space/views") + py_files(root, f"{API}/api/views"):
        try:
            t = ast.parse(f.read_text())
        except SyntaxError:
            continue
        for n in ast.walk(t):
            if isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and n.func.attr == "paginate":
                kws = {k.arg: ast.unparse(k.value)[:60] for k in n.keywords if k.arg}
                calls.append({"loc": f"{f.relative_to(root)}:{n.lineno}", "surface": f.relative_to(root).parts[3], "kwargs": sorted(kws),
                              "paginator_cls": kws.get("paginator_cls"), "default_per_page": kws.get("default_per_page")})
    out["paginate_call_sites"] = {
        "total": len(calls),
        "by_surface": dict(Counter(c["surface"] for c in calls)),
        "with_group_by": sum(1 for c in calls if "group_by_field_name" in c["kwargs"]),
        "with_sub_group_by": sum(1 for c in calls if "sub_group_by_field_name" in c["kwargs"]),
        "paginator_cls": dict(Counter(str(c["paginator_cls"]) for c in calls)),
        "default_per_page": dict(Counter(str(c["default_per_page"]) for c in calls)),
        "sites": calls,
    }

    # ------------------------------------------------------------------ C. query params the server reads
    qp: dict = {}
    for f in py_files(root, f"{API}/app/views"):
        keys = sorted(set(re.findall(r"""(?:request\.GET|request\.query_params|query_params)\.get\(\s*["'](\w+)["']""", f.read_text())))
        if keys:
            qp[str(f.relative_to(root))] = keys
    out["server_query_params_by_view_file"] = qp
    legacy = sorted(set(re.findall(r"""params\.get\(\s*["'](\w+)["']""", rd(root, f"{API}/utils/issue_filters.py"))))
    out["legacy_issue_filter_params"] = {"file": f"{API}/utils/issue_filters.py", "count": len(legacy), "params": legacy}
    out["rich_filter"] = {
        "backend": grep(root, f"{API}/utils/filters/filter_backend.py", r"filter_param\s*=|default_max_depth\s*="),
        "readme": _L.optional(f"{API}/utils/filters/README.md") is not None,
    }
    tparams = resolve_string_union(root, "TIssueParams")
    out["client_TIssueParams"] = {"count": len(tparams or []), "values": tparams}
    out["client_issue_param_keys_literal"] = sorted(set(re.findall(
        r"""(?:^|\s)(group_by|sub_group_by|order_by|sub_issue|cursor|per_page|expand|layout|filters|target_date|type|start_target_date)\s*[:=]""",
        rd(root, "apps/web/core/store/issue/helpers/issue-filter-helper.store.ts"))))

    # ------------------------------------------------------------------ E. expand / fields
    exp = {"server_property": grep(root, f"{API}/app/views/base.py", r"def (expand|fields)\(self\)")}
    n_exp = n_fields = 0
    for f in py_files(root, f"{API}/app/views"):
        t = f.read_text()
        n_exp += len(re.findall(r"self\.expand\b|GET\.get\(\s*[\"']expand[\"']", t))
        n_fields += len(re.findall(r"self\.fields\b|GET\.get\(\s*[\"']fields[\"']", t))
    exp["server_uses_expand"], exp["server_uses_fields"] = n_exp, n_fields
    vals: Counter = Counter()
    for f in walk_files(root / _L.resolve_dir("apps/web"), {".ts", ".tsx"}):
        for m in re.finditer(r"""\bexpand\s*[:=]\s*["'`]([A-Za-z_,]+)["'`]|[?&]expand=([A-Za-z_,]+)""", f.read_text(errors="ignore")):
            vals[m.group(1) or m.group(2)] += 1
    exp["client_expand_values"] = dict(vals)
    out["expand_fields"] = exp

    # ------------------------------------------------------------------ F. error bodies
    err = {
        "base_viewset_handle_exception": grep(root, f"{API}/app/views/base.py", r"def handle_exception|\"error\":"),
        "rest_framework_exception_handler": grep(root, f"{API}/settings/common.py", r"EXCEPTION_HANDLER"),
        "auth_exception_handler": grep(root, f"{API}/authentication/adapter/exception.py", r"def auth_exception_handler|isinstance\(exc, (NotAuthenticated|Throttled)\)|status_code = "),
    }
    counts = Counter()
    for f in py_files(root, f"{API}/app/views"):
        t = f.read_text()
        counts["Response({'error': ...})"] += len(re.findall(r"""Response\(\s*\{\s*["']error["']""", t))
        counts["Response({'message': ...})"] += len(re.findall(r"""Response\(\s*\{\s*["']message["']""", t))
        counts["Response({'detail': ...})"] += len(re.findall(r"""Response\(\s*\{\s*["']detail["']""", t))
        counts["Response(serializer.errors)"] += len(re.findall(r"Response\(\s*serializer\.errors", t))
        counts["raise ValidationError/ParseError/PermissionDenied (DRF {detail})"] += len(re.findall(r"raise\s+(?:ParseError|PermissionDenied|NotFound|ValidationError)\(", t))
    err["app_views_response_body_shapes"] = dict(counts)
    codes = {}
    err_rel = _L.resolve(f"{API}/authentication/adapter/error.py")  # loud if missing
    t = ast.parse(rd(root, err_rel))
    for n in t.body:
        if isinstance(n, ast.Assign) and any(getattr(x, "id", "") == "AUTHENTICATION_ERROR_CODES" for x in n.targets):
            codes = {k.value: v.value for k, v in zip(n.value.keys, n.value.values) if isinstance(k, ast.Constant)}
            err["auth_error_codes_loc"] = f"{err_rel}:{n.lineno}"
    if not codes:
        raise layout.LayoutError(f"AUTHENTICATION_ERROR_CODES not found in {err_rel}")
    err["auth_error_code_count"] = len(codes)
    err["auth_error_codes"] = codes
    out["error_bodies"] = err

    # ------------------------------------------------------------------ G. cookies / csrf / session
    ck = {}
    for name in ("SESSION_COOKIE_NAME", "SESSION_COOKIE_AGE", "SESSION_COOKIE_HTTPONLY", "SESSION_COOKIE_SECURE", "SESSION_COOKIE_DOMAIN", "SESSION_ENGINE",
                 "SESSION_SAVE_EVERY_REQUEST", "ADMIN_SESSION_COOKIE_NAME", "CSRF_COOKIE_HTTPONLY", "CSRF_COOKIE_SECURE", "CSRF_COOKIE_DOMAIN",
                 "CSRF_TRUSTED_ORIGINS", "CSRF_FAILURE_VIEW", "CORS_ALLOW_CREDENTIALS", "CORS_ALLOW_HEADERS", "APPEND_SLASH"):
        ck[name] = grep(root, f"{API}/settings/common.py", r"^" + name + r"\b", expect=0 if name == "APPEND_SLASH" else 1)
    ck["middleware"] = grep(root, f"{API}/settings/common.py", r"^\s+\"(plane|django|corsheaders|crum|whitenoise)[\w.]*(Middleware|middleware[\w.]*)\"")
    ck["drf_session_auth_no_csrf"] = grep(root, f"{API}/authentication/session.py", r"def enforce_csrf|return")
    ck["throttle_rates"] = grep(root, f"{API}/settings/common.py", r"\"(anon|asset_id)\": \"")
    ck["csrf_token_endpoint"] =grep(root, f"{API}/authentication/views/common.py", r"class CSRFTokenEndpoint|get_token|csrf_token")
    ck["client_axios_withCredentials_and_401_redirect"] = grep(root, "apps/web/core/services/api.service.ts", r"withCredentials|status === 401|location\.replace")
    ck["client_X_CSRFTOKEN_sites"] = sum(1 for _ in grep(root, "apps/web/core/services/auth.service.ts", r"X-CSRFTOKEN")) + sum(
        1 for _ in grep(root, "apps/web/core/services/user.service.ts", r"X-CSRFTOKEN"))
    cs = 0
    for f in walk_files(root / _L.resolve_dir("apps"), {".tsx"}):
        cs += len(re.findall(r"csrfmiddlewaretoken", f.read_text(errors="ignore")))
    ck["client_csrfmiddlewaretoken_field_sites"] = cs
    out["cookies_csrf_session"] = ck

    # ------------------------------------------------------------------ H. /auth/* redirect flow
    au: dict = {}
    classes = 0
    redirect_returns = 0
    next_path_reads = 0
    for f in py_files(root, f"{API}/authentication/views"):
        t = f.read_text()
        classes += len(re.findall(r"^class \w+\((?:View|APIView)\)", t, re.M))
        redirect_returns += len(re.findall(r"HttpResponseRedirect\(", t))
        next_path_reads += len(re.findall(r"""(?:POST|GET)\.get\(\s*["']next_path["']""", t))
    au["view_classes_View_or_APIView"] = classes
    au["HttpResponseRedirect_sites"] = redirect_returns
    au["next_path_read_sites"] = next_path_reads
    au["get_safe_redirect_url"] = grep(root, f"{API}/utils/path_validator.py", r"def get_safe_redirect_url")
    au["base_host"] = grep(root, f"{API}/authentication/utils/host.py", r"def base_host|settings\.(WEB_URL|APP_BASE_URL)")
    au["error_dict"] = grep(root, f"{API}/authentication/adapter/error.py", r"def get_error_dict|\"error_code\"")
    au["login_sets_session"] = grep(root, f"{API}/authentication/utils/login.py", r"login\(request|session\[")
    au["redirection_path_targets"] = grep(root, f"{API}/authentication/utils/redirection_path.py", r"return ")
    au["client_error_code_readers"] = grep(root, "apps/web/core/components/account/auth-forms/auth-root.tsx", r"error_code")
    out["auth_flow"] = au

    # ------------------------------------------------------------------ I. other wire quirks
    q: dict = {}
    redirects = []
    for f in py_files(root, f"{API}/app/views/asset") + py_files(root, f"{API}/space/views"):
        t = ast.parse(f.read_text())
        for n in ast.walk(t):
            if isinstance(n, ast.ClassDef):
                for fn in n.body:
                    if isinstance(fn, ast.FunctionDef) and "HttpResponseRedirect" in ast.get_source_segment(f.read_text(), fn):
                        redirects.append(f"{f.relative_to(root)}:{fn.lineno} {n.name}.{fn.name}")
    q["asset_handlers_returning_302_redirect"] = redirects
    q["gzip_page_decorated_handlers_in_app_views"] = sum(
        len(re.findall(r"method_decorator\(gzip_page\)", f.read_text())) for f in py_files(root, f"{API}/app/views"))
    q["gzip_middleware"] = grep(root, f"{API}/settings/common.py", r"GZipMiddleware")
    q["users_me_cache_headers"] = grep(root, f"{API}/app/views/user/base.py", r"cache_control|vary_on_cookie")
    q["instance_cache"] = grep(root, f"{API}/license/api/views/instance.py", r"cache_response|cache_control")
    q["web_not_found_handler"] = grep(root, f"{API}/urls.py", r"handler404")
    out["other_quirks"] = q
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    a = ap.parse_args()
    res = run(a.root.resolve())
    a.out.write_text(json.dumps(res, indent=1, sort_keys=True) + "\n")
    print(json.dumps({k: (v if not isinstance(v, dict) else sorted(v)) for k, v in res.items()}, indent=1)[:1500])


if __name__ == "__main__":
    main()
