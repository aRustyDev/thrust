# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Step 1 - server routes WITHOUT running Django.

Walks `apps/api/plane/urls.py` (ROOT_URLCONF) with Python's `ast`, resolving:
  * include("dotted.module") prefixes, list concatenation (`a + b`), starred sub-lists
    (`*alias_of_other_module.urlpatterns`), DRF `DefaultRouter` + `router.register` + `include(router.urls)`;
  * `View.as_view({...})` method maps (viewset action -> HTTP method), `as_view(http_method_names=[...])`,
    plain APIView classes (HTTP methods = handler methods defined on the class / in-repo bases);
  * path converters (`<str:slug>`, `<uuid:pk>`, mixed `<a>-<b>` segments) -> `{name}` and a positional `{}` form.
One output row per (HTTP method, normalised path), plus one row per `path()` entry.

Usage:  uv run server_routes.py --root <plane tree root> --out <dir>
"""

from __future__ import annotations

import argparse
import ast
import csv
import json
import re
import sys
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import layout  # noqa: E402
from pyindex import (  # noqa: E402
    DRF_GENERIC_VIEW_METHODS,
    DRF_VIEWSET_DEFAULTS,
    HTTP_METHODS,
    ClassInfo,
    PyIndex,
)

CONVERTER_RE = re.compile(r"<(?:(?P<conv>\w+):)?(?P<name>\w+)>")

# surface name by the include prefix declared in plane/urls.py
SURFACE_BY_PREFIX = {
    "api/": "internal",  # plane.app.urls  (the React app's API; NO schema)
    "api/public/": "space",  # plane.space.urls (published boards)
    "api/instances/": "instance",  # plane.license.urls (instance info + god-mode)
    "api/v1/": "v1",  # plane.api.urls (public REST API; drf-spectacular covers only this)
    "auth/": "auth",  # plane.authentication.urls (form-post / redirect flows + csrf token)
    "": "web",  # plane.web.urls (robots, health)
}

# aggregation / computed-column constructs: a handler that uses several is not a CRUD passthrough
AGG_CALLS = {"annotate", "aggregate", "Subquery", "OuterRef", "Exists", "Window", "Case", "When", "Count", "Sum", "Avg",
             "ArrayAgg", "Coalesce", "RawSQL", "raw", "extra", "values_list"}

ORM_CALLS = {
    "filter", "exclude", "annotate", "aggregate", "values", "values_list", "select_related", "prefetch_related",
    "get", "create", "bulk_create", "bulk_update", "update", "delete", "get_or_create", "update_or_create",
    "order_by", "distinct", "count", "exists", "first", "save",
}


@dataclass
class Pat:
    """A `path()` entry, or an `include()` wrapper."""

    route: str
    file: Path
    lineno: int
    view: ast.expr | None = None
    name: str | None = None
    children: list = field(default_factory=list)  # for include
    router_route: dict | None = None  # filled for router-generated entries
    is_include: bool = False


class UrlconfWalker:
    def __init__(self, ix: PyIndex):
        self.ix = ix
        self.warnings: list[str] = []

    # ---------------------------------------------------------------- evaluation of urlpatterns
    def eval_module_var(self, modname: str, var: str, _seen=None) -> list[Pat]:
        f = self.ix.module_file(modname)
        if f is None:
            self.warnings.append(f"module not found: {modname}")
            return []
        return self.eval_name(f, var, _seen or set())

    def eval_name(self, f: Path, name: str, _seen: set) -> list[Pat]:
        if (f, name) in _seen:
            return []
        _seen = _seen | {(f, name)}
        pats: list[Pat] = []
        found = False
        # direct assignments / augmented assignments (also inside `if` blocks, for conditional routes)
        for node in ast.walk(self.ix.tree(f)):
            if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == name for t in node.targets):
                pats += self.eval_expr(f, node.value, _seen)
                found = True
            elif isinstance(node, ast.AugAssign) and isinstance(node.target, ast.Name) and node.target.id == name:
                pats += self.eval_expr(f, node.value, _seen)
                found = True
        if found:
            return pats
        imp = self.ix.import_table(f)
        if name in imp:
            mod, attr = imp[name]
            if attr is not None:
                return self.eval_module_var(mod, attr, _seen)
        self.warnings.append(f"cannot resolve name {name} in {f}")
        return []

    def eval_expr(self, f: Path, e: ast.expr, _seen: set) -> list[Pat]:
        if isinstance(e, (ast.List, ast.Tuple)):
            out: list[Pat] = []
            for el in e.elts:
                if isinstance(el, ast.Starred):
                    out += self.eval_expr(f, el.value, _seen)
                else:
                    out += self.eval_expr(f, el, _seen)
            return out
        if isinstance(e, ast.BinOp) and isinstance(e.op, ast.Add):
            return self.eval_expr(f, e.left, _seen) + self.eval_expr(f, e.right, _seen)
        if isinstance(e, ast.Name):
            return self.eval_name(f, e.id, _seen)
        if isinstance(e, ast.Attribute) and e.attr == "urls" and isinstance(e.value, ast.Name):
            return self.router_patterns(f, e.value.id)
        if isinstance(e, ast.Call):
            fn = e.func.id if isinstance(e.func, ast.Name) else getattr(e.func, "attr", "")
            if fn in ("path", "re_path"):
                return [self.make_pat(f, e, fn)]
            self.warnings.append(f"{f}:{e.lineno}: unhandled call {ast.unparse(e)[:60]}")
            return []
        self.warnings.append(f"{f}:{getattr(e, 'lineno', '?')}: unhandled expr {type(e).__name__}")
        return []

    def make_pat(self, f: Path, call: ast.Call, fn: str) -> Pat:
        route = call.args[0]
        if not (isinstance(route, ast.Constant) and isinstance(route.value, str)):
            self.warnings.append(f"{f}:{call.lineno}: non-literal route")
            route_s = "<?>"
        else:
            route_s = route.value
        if fn == "re_path":
            self.warnings.append(f"{f}:{call.lineno}: re_path used ({route_s})")
        view = call.args[1] if len(call.args) > 1 else None
        name = None
        for kw in call.keywords:
            if kw.arg == "name" and isinstance(kw.value, ast.Constant):
                name = kw.value.value
        p = Pat(route=route_s, file=f, lineno=call.lineno, view=view, name=name)
        # include(...)
        if isinstance(view, ast.Call) and isinstance(view.func, ast.Name) and view.func.id == "include":
            arg = view.args[0]
            p.is_include = True
            if isinstance(arg, ast.Constant) and isinstance(arg.value, str):
                p.children = self.eval_module_var(arg.value, "urlpatterns")
            else:
                p.children = self.eval_expr(f, arg, set())
                if not p.children:
                    self.warnings.append(f"{f}:{call.lineno}: include() of non-project object skipped: {ast.unparse(arg)}")
            p.view = None
        return p

    # ---------------------------------------------------------------- DRF routers
    def router_patterns(self, f: Path, router: str) -> list[Pat]:
        out: list[Pat] = []
        regs = []
        for node in self.ix.tree(f).body:
            if (
                isinstance(node, ast.Expr)
                and isinstance(node.value, ast.Call)
                and isinstance(node.value.func, ast.Attribute)
                and node.value.func.attr == "register"
                and isinstance(node.value.func.value, ast.Name)
                and node.value.func.value.id == router
            ):
                regs.append(node.value)
        cls_name = None
        for node in self.ix.tree(f).body:
            if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == router for t in node.targets):
                if isinstance(node.value, ast.Call):
                    cls_name = ast.unparse(node.value.func)
        # DefaultRouter adds an API-root view at "" (APIRootView, GET) unless SimpleRouter
        if cls_name and cls_name.endswith("DefaultRouter"):
            out.append(Pat(route="", file=f, lineno=regs[0].lineno if regs else 0, name="api-root",
                           router_route={"kind": "api-root", "methods": ["get"], "class": "rest_framework.routers.APIRootView"}))
        for c in regs:
            prefix = c.args[0].value if isinstance(c.args[0], ast.Constant) else "<?>"
            vs = c.args[1]
            basename = None
            for kw in c.keywords:
                if kw.arg == "basename" and isinstance(kw.value, ast.Constant):
                    basename = kw.value.value
            ci = self._class_from_expr(f, vs)
            lookup = "pk"
            if ci:
                la = self.ix.class_attr(ci, "lookup_field")
                if la and isinstance(la[1], ast.Constant):
                    lookup = la[1].value
            for kind, route, method_map in (
                ("list", f"{prefix}/", {"get": "list", "post": "create"}),
                ("detail", f"{prefix}/<{lookup}>/",
                 {"get": "retrieve", "put": "update", "patch": "partial_update", "delete": "destroy"}),
            ):
                avail = {m: a for m, a in method_map.items() if ci and self._has_action(ci, a)}
                if avail:
                    out.append(Pat(route=route, file=f, lineno=c.lineno, name=f"{basename}-{kind}",
                                   router_route={"kind": kind, "map": avail, "class_expr": ast.unparse(vs)}))
            # @action extras
            if ci:
                for c2 in self.ix.mro(ci):
                    if not isinstance(c2, ClassInfo):
                        continue
                    for mname, fn in c2.methods.items():
                        for dec in fn.decorator_list:
                            if isinstance(dec, ast.Call) and getattr(dec.func, "id", getattr(dec.func, "attr", "")) == "action":
                                kw = {k.arg: k.value for k in dec.keywords}
                                detail = isinstance(kw.get("detail"), ast.Constant) and kw["detail"].value
                                methods = [m.value for m in kw["methods"].elts] if "methods" in kw else ["get"]
                                url_path = kw["url_path"].value if "url_path" in kw else mname
                                route = f"{prefix}/<{lookup}>/{url_path}/" if detail else f"{prefix}/{url_path}/"
                                out.append(Pat(route=route, file=f, lineno=c.lineno, name=f"{basename}-{mname}",
                                               router_route={"kind": "action", "map": {m.lower(): mname for m in methods},
                                                             "class_expr": ast.unparse(vs)}))
        return out

    def _class_from_expr(self, f: Path, e: ast.expr) -> ClassInfo | None:
        if isinstance(e, ast.Name):
            r = self.ix.resolve_name_in_file(f, e.id)
            if r and r[0] == "class":
                return r[1]
        return None

    def _has_action(self, ci: ClassInfo, action: str) -> bool:
        if self.ix.find_method(ci, action):
            return True
        for b in self.ix.external_bases(ci):
            if action in DRF_VIEWSET_DEFAULTS.get(b, []):
                return True
        return False


# =================================================================================== flatten & analyse
def normalise(route: str) -> tuple[str, str, list[dict]]:
    """-> (named form `/a/{slug}/`, positional form `/a/{}/`, params[])"""
    params = [{"name": m.group("name"), "converter": m.group("conv") or "str"} for m in CONVERTER_RE.finditer(route)]
    named = CONVERTER_RE.sub(lambda m: "{" + m.group("name") + "}", route)
    pos = CONVERTER_RE.sub("{}", route)
    return named, pos, params


def handler_metrics(ix: PyIndex, fn: ast.FunctionDef | None, owner: ClassInfo | None) -> dict:
    if fn is None or owner is None:
        return {}
    src_lines = ix.source(owner.file).splitlines()
    end = getattr(fn, "end_lineno", fn.lineno)
    seg = "\n".join(src_lines[fn.lineno - 1 : end])
    attrs_called: dict[str, int] = {}
    sers: list[str] = []
    for n in ast.walk(fn):
        if isinstance(n, ast.Call):
            nm = n.func.attr if isinstance(n.func, ast.Attribute) else getattr(n.func, "id", None)
            if nm:
                attrs_called[nm] = attrs_called.get(nm, 0) + 1
        if isinstance(n, ast.Name) and n.id.endswith("Serializer") and n.id not in sers:
            sers.append(n.id)
    return {
        "loc": end - fn.lineno + 1,
        "decorators": [ast.unparse(d) for d in fn.decorator_list],
        "agg_calls": sum(v for k, v in attrs_called.items() if k in AGG_CALLS),
        "orm_calls": sum(v for k, v in attrs_called.items() if k in ORM_CALLS),
        "bg_tasks": attrs_called.get("delay", 0),
        "paginates": attrs_called.get("paginate", 0) > 0,
        "mentions_group_by": "group_by" in seg,
        "mentions_bulk": bool(re.search(r"bulk_(create|update|delete)|bulk", seg)),
        "mentions_storage": bool(re.search(r"S3Storage|presigned|generate_presigned|storage\.|FileAsset", seg)),
        "mentions_redirect": bool(re.search(r"redirect|HttpResponseRedirect", seg)),
        "serializers": sers,
    }


def serializer_info(ix: PyIndex, owner_file: Path, name: str) -> dict | None:
    r = ix.resolve_name_in_file(owner_file, name)
    if not r or r[0] != "class":
        return None
    ci: ClassInfo = r[1]
    declared = 0
    meta_fields = None
    for st in ci.node.body:
        if isinstance(st, ast.Assign) and isinstance(st.value, ast.Call):
            declared += 1
        if isinstance(st, ast.ClassDef) and st.name == "Meta":
            for m in st.body:
                if isinstance(m, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "fields" for t in m.targets):
                    v = m.value
                    if isinstance(v, ast.Constant):
                        meta_fields = v.value  # "__all__"
                    elif isinstance(v, (ast.List, ast.Tuple)):
                        meta_fields = len(v.elts)
    return {"loc": ci.loc(ix.root), "declared_fields": declared, "meta_fields": meta_fields}


def analyse_view(ix: PyIndex, pat: Pat) -> dict:
    """Resolve pat.view (or router_route) -> {kind, class, class_loc, methods:{http:{action,handler_loc,...}}}"""
    f = pat.file
    info: dict = {"kind": "unresolved", "class": None, "class_loc": None, "methods": {}, "restricted_by": None,
                  "external_bases": [], "permission_classes": None, "authentication_classes": None}
    ci = None
    method_map = None
    http_names = None
    if pat.router_route:
        rr = pat.router_route
        if rr["kind"] == "api-root":
            info.update(kind="drf-router-root", **{"class": "rest_framework.routers.APIRootView"})
            info["methods"] = {"get": {"action": "get", "handler_loc": None, "drf_default": True}}
            return info
        ci = UrlconfWalker(ix)._class_from_expr(f, ast.parse(rr["class_expr"], mode="eval").body)
        method_map = rr["map"]
        info["kind"] = f"drf-router-{rr['kind']}"
    elif isinstance(pat.view, ast.Call) and isinstance(pat.view.func, ast.Attribute) and pat.view.func.attr == "as_view":
        target = pat.view.func.value
        tname = target.id if isinstance(target, ast.Name) else (target.attr if isinstance(target, ast.Attribute) else None)
        r = ix.resolve_name_in_file(f, tname) if isinstance(target, ast.Name) else None
        if r and r[0] == "class":
            ci = r[1]
        info["class"] = tname
        if pat.view.args and isinstance(pat.view.args[0], ast.Dict):
            d = pat.view.args[0]
            method_map = {k.value: v.value for k, v in zip(d.keys, d.values)
                          if isinstance(k, ast.Constant) and isinstance(v, ast.Constant)}
            info["kind"] = "viewset-map"
        else:
            info["kind"] = "apiview"
        for kw in pat.view.keywords:
            if kw.arg == "http_method_names" and isinstance(kw.value, (ast.List, ast.Tuple)):
                http_names = [e.value for e in kw.value.elts if isinstance(e, ast.Constant)]
                info["restricted_by"] = http_names
    elif isinstance(pat.view, ast.Name):
        r = ix.resolve_name_in_file(f, pat.view.id)
        info.update(kind="function-view", **{"class": pat.view.id})
        if r and r[0] == "func":
            info["class_loc"] = f"{r[1].relative_to(ix.root)}:{r[2].lineno}"
            info["methods"] = {"any": {"action": pat.view.id, "handler_loc": info["class_loc"],
                                       **handler_metrics(ix, r[2], ClassInfo(pat.view.id, r[1], r[2].lineno, None, "", [], {}))}}
        else:
            info["methods"] = {"any": {"action": pat.view.id, "handler_loc": None}}
        return info
    else:
        return info

    if ci is None:
        # third-party view class (e.g. drf_spectacular): assume GET
        info["methods"] = {"get": {"action": "get", "handler_loc": None, "external": True}}
        info["external_bases"] = ["<external view class>"]
        return info

    info["class"] = ci.name
    info["class_loc"] = ci.loc(ix.root)
    info["external_bases"] = ix.external_bases(ci)
    for attr in ("permission_classes", "authentication_classes"):
        a = ix.class_attr(ci, attr)
        info[attr] = ast.unparse(a[1]) if a else None

    def one(http: str, action: str):
        found = ix.find_method(ci, action)
        if found:
            owner, fn = found
            m = {"action": action, "handler_loc": f"{owner.file.relative_to(ix.root)}:{fn.lineno}",
                 "handler_owner": owner.name, **handler_metrics(ix, fn, owner)}
            # decorate with serializer detail
            m["serializer_info"] = [x for x in (dict(name=s, **(serializer_info(ix, owner.file, s) or {}))
                                                for s in m.get("serializers", [])[:4])]
            return m
        for b in ix.external_bases(ci):
            if action in DRF_VIEWSET_DEFAULTS.get(b, []) or http in DRF_GENERIC_VIEW_METHODS.get(b, []):
                # DRF default action: cost sits in get_queryset/serializer_class of the project class
                gq = ix.find_method(ci, "get_queryset")
                m = {"action": action, "handler_loc": None, "drf_default": True, "via": b}
                if gq:
                    m.update(handler_metrics(ix, gq[1], gq[0]))
                    m["handler_loc"] = f"{gq[0].file.relative_to(ix.root)}:{gq[1].lineno} (get_queryset)"
                return m
        return {"action": action, "handler_loc": None, "missing": True}

    if method_map is not None:
        for http, action in method_map.items():
            info["methods"][http.lower()] = one(http.lower(), action)
    else:
        defined = [m for m in HTTP_METHODS if ix.find_method(ci, m)]
        for b in ix.external_bases(ci):
            for m in DRF_GENERIC_VIEW_METHODS.get(b, []):
                if m not in defined:
                    defined.append(m)
        if http_names is not None:
            allowed = {h.lower() for h in http_names}
            defined = [m for m in defined if m in allowed]
            for h in allowed - set(defined) - {"options", "head"}:
                info.setdefault("declared_but_undefined", []).append(h)
        for m in defined:
            info["methods"][m] = one(m, m)
    return info


def flatten(pats: list[Pat], prefix: str, chain: list[str]):
    for p in pats:
        full = prefix + p.route
        if p.is_include:
            yield from flatten(p.children, full, chain + [f"{p.file.name}:{p.lineno}"])
        else:
            yield p, full, chain


def run(root: Path) -> dict:
    ix = PyIndex(root)
    w = UrlconfWalker(ix)
    L = layout.for_root(root)
    L.resolve("apps/api/plane/urls.py")  # loud if --root is not a Plane tree (or the API moved)
    root_urls = ix.module_file("plane.urls")
    assert root_urls, "plane/urls.py not importable by module name: PyIndex.pyroot is wrong for this layout"
    top = w.eval_module_var("plane.urls", "urlpatterns")
    entries: list[dict] = []
    rows: list[dict] = []
    for p, full, chain in flatten(top, "", []):
        # surface = longest matching include prefix of the root conf
        surface, sprefix = "other", ""
        for pref in sorted(SURFACE_BY_PREFIX, key=len, reverse=True):
            if pref and full.startswith(pref):
                surface, sprefix = SURFACE_BY_PREFIX[pref], pref
                break
        else:
            if "" in SURFACE_BY_PREFIX:
                surface = SURFACE_BY_PREFIX[""]
        conditional = p.file.name == "urls.py" and p.file.parent.name == "plane" and p.name in ("schema", "swagger-ui", "redoc")
        if conditional:
            surface = "schema"
        info = analyse_view(ix, p)
        named, pos, params = normalise("/" + full)
        entry = {
            "surface": surface,
            "route_entry_loc": f"{p.file.relative_to(root)}:{p.lineno}",
            "full_path": named,
            "norm_path": pos,
            "params": params,
            "url_name": p.name,
            "view_kind": info["kind"],
            "view_class": info["class"],
            "view_class_loc": info["class_loc"],
            "methods": sorted(info["methods"].keys()),
            "conditional": "settings.ENABLE_DRF_SPECTACULAR" if conditional else "",
            "permission_classes": info.get("permission_classes"),
            "authentication_classes": info.get("authentication_classes"),
            "external_bases": info["external_bases"],
            "http_method_names_restriction": info.get("restricted_by"),
            "declared_but_undefined": info.get("declared_but_undefined", []),
        }
        entries.append(entry)
        for http, m in sorted(info["methods"].items()):
            rows.append({
                "surface": surface,
                "method": http.upper(),
                "full_path": named,
                "norm_path": pos,
                "params": ",".join(f"{x['name']}:{x['converter']}" for x in params),
                "view_class": info["class"],
                "view_class_loc": info["class_loc"],
                "route_entry_loc": entry["route_entry_loc"],
                "url_name": p.name,
                "view_kind": info["kind"],
                "action": m.get("action"),
                "handler_loc": m.get("handler_loc"),
                "handler_loc_n": m.get("loc"),
                "orm_calls": m.get("orm_calls"),
                "agg_calls": m.get("agg_calls"),
                "bg_tasks": m.get("bg_tasks"),
                "paginates": m.get("paginates"),
                "mentions_group_by": m.get("mentions_group_by"),
                "mentions_bulk": m.get("mentions_bulk"),
                "mentions_storage": m.get("mentions_storage"),
                "mentions_redirect": m.get("mentions_redirect"),
                "serializers": ",".join(m.get("serializers", [])),
                "serializer_fields": ";".join(
                    f"{s['name']}={s.get('meta_fields')}/{s.get('declared_fields')}" for s in m.get("serializer_info", []) if "loc" in s
                ),
                "decorators": " | ".join(m.get("decorators", [])),
                "drf_default": bool(m.get("drf_default")),
                "permission_classes": info.get("permission_classes") or "",
                "authentication_classes": info.get("authentication_classes") or "",
                "conditional": entry["conditional"],
            })
    return {"entries": entries, "rows": rows, "warnings": w.warnings, "reconciliation": reconcile(root, entries)}


def reconcile(root: Path, entries: list[dict]) -> dict:
    """Raw `path(` lines per URL conf file vs entries actually reachable from ROOT_URLCONF (explains review-count gaps)."""
    L = layout.for_root(root)
    py = root / "apps" / "api" / "plane"
    globbed = []
    for d in ("app/urls", "space/urls", "api/urls"):
        found = sorted((root / L.resolve_dir(f"apps/api/plane/{d}")).glob("*.py"))
        if not found:
            raise layout.LayoutError(f"no URL conf files in apps/api/plane/{d}")
        globbed.append(found)
    single = [root / L.resolve(f"apps/api/plane/{f}") for f in ("authentication/urls.py", "license/urls.py", "web/urls.py", "urls.py")]
    files = globbed[0] + globbed[1] + [single[0]] + globbed[2] + single[1:]
    reached: Counter = Counter(e["route_entry_loc"].rsplit(":", 1)[0] for e in entries)
    # a URL conf that is reachable but not in the fixed list above must still appear (new apps / moved confs)
    listed = {str(f.relative_to(root)) for f in files}
    files += [root / rel for rel in sorted(reached) if rel not in listed]
    rows = {}
    for f in files:
        rel = str(f.relative_to(root))
        raw = sum(1 for ln in f.read_text().splitlines() if re.match(r"^\s*(?:re_)?path\(", ln))
        rows[rel] = {"raw_path_lines": raw, "reachable_entries": reached.get(rel, 0)}
    by_dir: dict[str, dict] = {}
    for rel, v in rows.items():
        d = rel.rsplit("/", 1)[0] if "/urls/" in rel else rel
        agg = by_dir.setdefault(d, {"raw_path_lines": 0, "reachable_entries": 0, "files_never_reached": []})
        agg["raw_path_lines"] += v["raw_path_lines"]
        agg["reachable_entries"] += v["reachable_entries"]
        if v["raw_path_lines"] and not v["reachable_entries"]:
            agg["files_never_reached"].append(rel.rsplit("/", 1)[1])
    return {"per_file": rows, "per_dir": by_dir}


def summarise(res: dict) -> dict:
    ents, rows = res["entries"], res["rows"]
    surfaces = sorted({e["surface"] for e in ents})
    out = {}
    for s in surfaces:
        se = [e for e in ents if e["surface"] == s]
        sr = [r for r in rows if r["surface"] == s]
        out[s] = {
            "route_entries": len(se),
            "endpoint_rows_method_x_path": len(sr),
            "distinct_norm_paths": len({r["norm_path"] for r in sr}),
            "by_method": {m: sum(1 for r in sr if r["method"] == m) for m in sorted({r["method"] for r in sr})},
        }
    # duplicates: same (method, norm_path) declared by >1 entry
    seen: dict[tuple, list] = {}
    for r in rows:
        seen.setdefault((r["surface"], r["method"], r["norm_path"]), []).append(r["route_entry_loc"])
    out["_duplicate_method_path"] = {" ".join(k): v for k, v in seen.items() if len(v) > 1}
    out["_reconciliation"] = res.get("reconciliation", {})
    compat = [r for r in rows if r["surface"] in ("internal", "space", "instance", "auth")]
    perm_rows = [r for r in compat if "allow_permission(" in (r["decorators"] or "")]
    out["_facts"] = {
        "rows_all_surfaces": len(rows),
        "rows_compat_surfaces": len(compat),
        "view_kind_all_rows": dict(Counter(r["view_kind"] for r in rows)),
        "drf_default_rows_all": sum(1 for r in rows if r["drf_default"]),
        "drf_default_rows_compat": sum(1 for r in compat if r["drf_default"]),
        "bg_task_rows_compat (.delay in handler body)": sum(1 for r in compat if (r["bg_tasks"] or 0) > 0),
        "paginate_rows_compat": sum(1 for r in compat if r["paginates"]),
        "group_by_rows_compat": sum(1 for r in compat if r["mentions_group_by"]),
        "gzip_page_rows_compat": sum(1 for r in compat if "gzip_page" in (r["decorators"] or "")),
        "allow_permission_rows_compat": len(perm_rows),
        "allow_permission_with_creator_rows_compat": sum(1 for r in perm_rows if "creator=True" in r["decorators"]),
        "class_permission_rows_compat": len(compat) - len(perm_rows),
        "cache_response_rows_compat": sum(1 for r in compat if "cache_response(" in (r["decorators"] or "")),
        "invalidate_cache_rows_compat": sum(1 for r in compat if "invalidate_cache(" in (r["decorators"] or "")),
    }
    return out


def write_outputs(res: dict, out: Path):
    out.mkdir(parents=True, exist_ok=True)
    (out / "server-routes.json").write_text(json.dumps(res["rows"], indent=1, sort_keys=True) + "\n")
    (out / "server-route-entries.json").write_text(json.dumps(res["entries"], indent=1, sort_keys=True) + "\n")
    cols = list(res["rows"][0].keys())
    with open(out / "server-routes.csv", "w", newline="") as fh:
        wr = csv.DictWriter(fh, fieldnames=cols)
        wr.writeheader()
        for r in sorted(res["rows"], key=lambda r: (r["surface"], r["norm_path"], r["method"])):
            wr.writerow(r)
    (out / "server-summary.json").write_text(json.dumps(summarise(res), indent=1, sort_keys=True) + "\n")
    (out / "server-warnings.txt").write_text("\n".join(sorted(set(res["warnings"]))) + "\n")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", required=True, type=Path, help="Plane tree root (contains apps/ and packages/)")
    ap.add_argument("--out", required=True, type=Path)
    a = ap.parse_args()
    res = run(a.root.resolve())
    write_outputs(res, a.out)
    print(json.dumps(summarise(res), indent=1, sort_keys=True))


if __name__ == "__main__":
    main()
