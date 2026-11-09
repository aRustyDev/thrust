"""Enumerate every /api/v1 operation (path x method) from the LIVE Django URL resolver.

Run INSIDE the api container through Django's shell:

    docker compose -f oracle/compose.oracle.yml --env-file oracle/.env.oracle exec -T api \
        python manage.py shell -c "exec(open('/oracle/urlconf_ops.py').read())"

Prints JSON between ORACLE_URLCONF_BEGIN / ORACLE_URLCONF_END. The host-side comparator
(oracle/harness/coverage_report.py) diffs this against the schema served at /api/schema/.

Why this is not just "list the methods on the view class":

* One class is often mounted on BOTH a list route and a detail route, so the class's methods
  are not all callable on every route. Each (route, method) is therefore checked against the
  handler's SIGNATURE: every required parameter must be supplied by the route's kwargs and every
  route kwarg must be accepted. A mismatch is a `phantom`: it is routed, but calling it raises
  TypeError inside the view (a 500), so it is not a usable operation. They are reported
  separately, never silently dropped and never counted as real operations.
* DRF routers emit regex routes (`^stickies/(?P<pk>[^/.]+)/$`), a `.format` suffix twin of each
  route, and an API-root view. These are translated to OpenAPI templates / tagged, not guessed.

`kind` is one of: path | router | router-format | router-root.
"""

import inspect
import json
import re

from django.urls import URLPattern, URLResolver, get_resolver

HTTP = ("get", "post", "put", "patch", "delete")
PREFIX = "api/v1/"
ANGLE = re.compile(r"<(?:\w+:)?(\w+)>")  # path(): <uuid:project_id>
NAMED_GROUP = re.compile(r"\(\?P<(\w+)>[^)]*\)")  # re_path()/router: (?P<pk>[^/.]+)

ops = []


def to_template(full):
    """Django route -> (OpenAPI template, kind, route kwarg names)."""
    kind = "path"
    s = full
    if "(?P<" in s or "^" in s or "$" in s:
        kind = "router"
    if "\\." in s and "(?P<format>" in s:
        kind = "router-format"
    # Named groups FIRST: `(?P<pk>[^/.]+)` also contains `<pk>`, which the angle-bracket
    # pattern would otherwise rewrite and leave the rest of the group behind.
    kwargs = NAMED_GROUP.findall(s)
    s = NAMED_GROUP.sub(r"{\1}", s)
    kwargs += ANGLE.findall(s)
    s = ANGLE.sub(r"{\1}", s)
    # Router regex machinery: a leading ^ / trailing $ anchor and the `/?` optional slash.
    # Router paths concatenate several anchored fragments (`api/v1/workspaces/<slug>/^stickies/$`),
    # so strip an anchor wherever it sits at a fragment boundary, not inside a character class.
    s = re.sub(r"(?<=/)\^", "", s)
    s = s.lstrip("^").rstrip("$").replace("/?", "/")
    s = re.sub(r"\\\.\{format\}[^/]*$", "", s)
    return "/" + s, kind, kwargs


def signature_ok(func, route_kwargs):
    """True if func(request, **route_kwargs) is callable without TypeError."""
    try:
        params = list(inspect.signature(func).parameters.values())
    except (TypeError, ValueError):
        return True, [], []
    params = params[1:]  # drop self
    if params and params[0].name == "request":
        params = params[1:]
    var_kw = any(p.kind is inspect.Parameter.VAR_KEYWORD for p in params)
    named = [p for p in params if p.kind in (p.POSITIONAL_OR_KEYWORD, p.KEYWORD_ONLY)]
    required = {p.name for p in named if p.default is p.empty}
    accepted = {p.name for p in named}
    missing = sorted(required - set(route_kwargs))
    unexpected = [] if var_kw else sorted(set(route_kwargs) - accepted)
    return (not missing and not unexpected), missing, unexpected


def walk(patterns, prefix):
    for p in patterns:
        route = str(p.pattern)
        if isinstance(p, URLResolver):
            walk(p.url_patterns, prefix + route)
            continue
        if not isinstance(p, URLPattern):
            continue
        full = prefix + route
        if not full.startswith(PREFIX):
            continue
        cb = p.callback
        cls = getattr(cb, "cls", None) or getattr(cb, "view_class", None)
        actions = getattr(cb, "actions", None)  # ViewSet routes: {"get": "list", ...}
        initkwargs = getattr(cb, "initkwargs", None) or {}
        templ, kind, route_kwargs = to_template(full)
        if cls is not None and cls.__name__ == "APIRootView":
            kind = "router-root"
        # Plane's v1 URL conf narrows each route with as_view(http_method_names=[...]); that
        # per-route list, not the class default, is what the server enforces (405 otherwise).
        allowed = initkwargs.get("http_method_names") or getattr(cls, "http_method_names", HTTP)
        if actions:
            handlers = {
                m: getattr(cls, actions[m], None) for m in HTTP if m in actions and m in allowed
            }
        elif cls is not None:
            handlers = {m: getattr(cls, m) for m in HTTP if m in allowed and hasattr(cls, m)}
        else:
            handlers = {}
        for m, fn in handlers.items():
            ok, missing, unexpected = signature_ok(fn, route_kwargs) if fn else (True, [], [])
            ops.append(
                {
                    "path": templ,
                    "method": m.upper(),
                    "view": f"{cls.__module__}.{cls.__qualname__}" if cls else str(cb),
                    "name": p.name,
                    "kind": kind,
                    "signature_ok": ok,
                    "sig_missing": missing,
                    "sig_unexpected": unexpected,
                }
            )


walk(get_resolver().url_patterns, "")
ops.sort(key=lambda o: (o["path"], o["method"], o["kind"]))
print("ORACLE_URLCONF_BEGIN")
print(json.dumps(ops, indent=1))
print("ORACLE_URLCONF_END")
