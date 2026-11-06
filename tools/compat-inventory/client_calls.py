# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Step 2 - client call sites (static, dependency-free).

Finds, in the React/TS sources:
  A. axios-style calls in APIService subclasses: `this.get|post|put|patch|delete|request(<path>, ...)`
     -> method, path template, inline query, `params:` keys, request body (arg + declared type),
        response type (`Promise<T>` of the enclosing method), `.then` cast, what `.catch` throws,
        request options (headers / validateStatus / responseType).
  B. every other literal that is a server path (`/api/...`, `/auth/...`) in code: `fetch(...)`,
     JSX `action=` form posts, `window.location.assign(...)`, `href=`/`src=`, `form.action = ...`,
     helper `getFileURL(...)` arguments, and plain constants.
  C. service-method usage counts (is the service method called from anywhere else?) as a liveness hint.

Path templates are normalised to the same placeholder form as the server (`{}` positional + `{name}`).
`${this.serviceType}` is expanded from the `EIssueServiceType` enum read from packages/types.
Paths that cannot be resolved statically are listed in `client-unresolved.json` and, where a
hand-resolution exists, taken from client_overrides.py (marked `override`).

Usage:  uv run client_calls.py --root <plane tree root> --out <dir>
"""

from __future__ import annotations

import argparse
import csv
import itertools
import json
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import layout  # noqa: E402
from client_overrides import BASE_EXPRS, OVERRIDES as _OVERRIDES_RAW, PLACEHOLDER_SUBST as _SUBST_RAW, THIS_ENUM_ALIASES  # noqa: E402  (hand-curated inputs)
from ts_lex import Tok, lex, match_close, split_top, src_of  # noqa: E402

VERBS = {"get", "post", "put", "patch", "delete", "request"}
SKIP_DIRS = {"node_modules", "build", "dist", ".next", ".react-router", ".turbo", "storybook-static", "coverage", ".git"}
EXTS = {".ts", ".tsx"}
CAP_ALTS = 12
# The hand-curated tables are keyed by file path as written in the pre-pin layout; compare through layout.canon()
# so they keep matching after upstream moves files (apps/web/core/** -> apps/web/**).
OVERRIDES = {(layout.canon(k[0]), k[1], k[2]): v for k, v in _OVERRIDES_RAW.items()}
PLACEHOLDER_SUBST = {(layout.canon(k[0]), k[1]): v for k, v in _SUBST_RAW.items()}
OVERRIDES_USED: set = set()

# (path under root, client label, requested-by-brief?)
DEFAULT_CLIENTS = [
    ("packages/services", "pkg-services", True),
    ("apps/web", "web", True),
    ("apps/space", "space", True),
    ("apps/admin", "admin", False),  # extra: god-mode app (ADR-0021 retires it)
    ("apps/live", "live", False),  # extra: collab server that is itself a client of the internal API
    ("packages/utils", "pkg-other", False),
    ("packages/editor", "pkg-other", False),
    ("packages/constants", "pkg-other", False),
    ("packages/hooks", "pkg-other", False),
    ("packages/shared-state", "pkg-other", False),
    ("packages/ui", "pkg-other", False),
    ("packages/propel", "pkg-other", False),
]
# Client dirs that may legitimately not exist (upstream removed / externalised them: `packages/ui` and `packages/propel` are
# gone at the oracle pin, propel now arrives as the npm package @makeplane/propel). A required dir that is missing is an error;
# an optional one is RECORDED as absent in layout-report.json. Nothing is skipped silently.
OPTIONAL_CLIENT_DIRS = {"packages/ui", "packages/propel"}


# ===================================================================== file discovery
def walk_files(base: Path, suffixes: set[str]):
    """Files under `base` with a given suffix, pruning SKIP_DIRS while walking, in the same order as sorted(base.rglob('*'))."""
    out = []
    for d, dirnames, filenames in os.walk(base):
        dirnames[:] = [x for x in dirnames if x not in SKIP_DIRS]
        for fn in filenames:
            p = Path(d) / fn
            if p.suffix in suffixes:
                out.append(p)
    return sorted(out)  # Path ordering (component-wise), identical to the earlier sorted(rglob) order


def iter_sources(root: Path, rel: str):
    L = layout.for_root(root)
    if rel in OPTIONAL_CLIENT_DIRS:
        used = L.optional(rel, kind="dir")
        if used is None:
            return
    else:
        used = L.resolve_dir(rel)  # loud: a required client dir that moved or vanished must stop the run
    for f in walk_files(root / used, EXTS):
        if f.name.endswith(".d.ts"):
            continue
        parts = set(f.relative_to(root).parts)
        n = f.name
        if ".stories." in n or ".spec." in n or ".test." in n or "__tests__" in parts or "tests" in parts:
            continue
        yield f


_API_LITERAL = re.compile(r"""["'`](/(?:api|auth)/[^"'`\s]*)""")


def unlisted_dirs(root: Path) -> list[dict]:
    """apps/* and packages/* directories that are NOT scanned as clients (new packages appear here after an upstream sync).

    Reports, per directory, how many quoted `/api/...` or `/auth/...` literals it contains outside comment lines, so a new
    package that starts calling the API is visible instead of silently ignored. `apps/api` (the server) is excluded."""
    listed = {rel for rel, _, _ in DEFAULT_CLIENTS}
    out = []
    for top in ("apps", "packages"):
        for d in sorted((root / top).iterdir()):
            rel = f"{top}/{d.name}"
            if not d.is_dir() or d.name in SKIP_DIRS or rel in listed or rel == "apps/api":
                continue
            hits, n_files = [], 0
            for f in walk_files(d, {".ts", ".tsx", ".js", ".jsx", ".mjs"}):
                n_files += 1
                for i, line in enumerate(f.read_text(encoding="utf-8", errors="ignore").splitlines(), 1):
                    s = line.strip()
                    if s.startswith("//") or s.startswith("*") or s.startswith("/*"):
                        continue
                    if _API_LITERAL.search(line):
                        hits.append(f"{f.relative_to(root)}:{i}")
            out.append({"dir": rel, "files": n_files, "api_literal_hits": len(hits), "first_hits": hits[:5]})
    return out


# ===================================================================== enum / union lookup
def resolve_enum(root: Path, enum_name: str) -> list[str]:
    pat = re.compile(r"export\s+(?:const\s+)?enum\s+" + re.escape(enum_name) + r"\s*\{(.*?)\}", re.S)
    types_dir = layout.for_root(root).resolve_dir("packages/types")
    for f in walk_files(root / types_dir, {".ts"}):
        if f.name.endswith(".d.ts"):
            continue
        m = pat.search(f.read_text(encoding="utf-8"))
        if m:
            return re.findall(r"=\s*\"([^\"]+)\"", m.group(1))
    # an enum that is not found would silently leave `${this.serviceType}` unexpanded: stop instead
    raise layout.LayoutError(f"enum {enum_name} not found under {types_dir} (client_overrides.THIS_ENUM_ALIASES is stale)")


_UNION_CACHE: dict[str, list[str] | None] = {}


def resolve_string_union(root: Path, type_name: str) -> list[str] | None:
    """`export type X = "a" | "b" | ...` in packages/types -> ["a","b"]."""
    if type_name in _UNION_CACHE:
        return _UNION_CACHE[type_name]
    out = None
    found = False
    pat = re.compile(r"export\s+type\s+" + re.escape(type_name) + r"\s*=\s*((?:[^;]|\n)*?);", re.S)
    types_dir = layout.for_root(root).resolve_dir("packages/types")
    for f in walk_files(root / types_dir, {".ts"}):
        if f.name.endswith(".d.ts"):
            continue
        m = pat.search(f.read_text(encoding="utf-8"))
        if m:
            found = True
            vals = re.findall(r"\"([^\"]+)\"", m.group(1))
            if vals and re.fullmatch(r"[\s|\"A-Za-z0-9_\-:.,/]*", m.group(1)):
                out = vals
            break
    if not found:
        raise layout.LayoutError(f"type {type_name} not found under {types_dir}")
    _UNION_CACHE[type_name] = out
    return out


# ===================================================================== structure parsing
@dataclass
class Member:
    name: str
    kind: str  # method | arrow
    line: int
    params_lo: int
    params_hi: int
    ret_lo: int
    ret_hi: int
    body_lo: int
    body_hi: int


@dataclass
class ClassDecl:
    name: str
    extends: str | None
    line: int
    body_lo: int
    body_hi: int
    members: list = field(default_factory=list)
    super_base: str | None = None


MODS = {"public", "private", "protected", "static", "readonly", "async", "abstract", "override", "get", "set"}


def find_classes(toks: list[Tok], src: str) -> list[ClassDecl]:
    out = []
    i = 0
    while i < len(toks):
        t = toks[i]
        if t.id("class") and i + 1 < len(toks) and toks[i + 1].kind == "id" and not (i > 0 and toks[i - 1].p(".")):
            name = toks[i + 1].text
            j = i + 2
            ext = None
            while j < len(toks) and not toks[j].p("{"):
                if toks[j].id("extends") and j + 1 < len(toks):
                    ext = toks[j + 1].text
                j += 1
            if j < len(toks):
                be = match_close(toks, j)
                cd = ClassDecl(name, ext, t.line, j + 1, be)
                cd.members = parse_members(toks, src, cd.body_lo, cd.body_hi)
                for m in cd.members:
                    if m.name == "constructor":
                        for k in range(m.body_lo, m.body_hi):
                            if toks[k].id("super") and k + 1 < m.body_hi and toks[k + 1].p("("):
                                ce = match_close(toks, k + 1)
                                cd.super_base = src_of(src, toks, k + 2, ce)
                out.append(cd)
                i = be
                continue
        i += 1
    return out


def skip_type_until_body(toks: list[Tok], j: int, hi: int) -> int:
    """From the token after `)` (maybe `:` return type) find the `{` / `=>` that starts the body."""
    angle = paren = 0
    while j < hi:
        t = toks[j]
        if t.kind == "punct":
            if t.text == "<":
                angle += 1
            elif t.text == ">":
                angle -= 1
            elif t.text in "([":
                paren += 1
            elif t.text in ")]":
                paren -= 1
            elif t.text == "{" and angle <= 0 and paren == 0:
                return j
            elif t.text == "=>" and angle <= 0 and paren == 0:
                return j
            elif t.text == ";" and paren == 0:
                return j
            elif t.text == "{":
                j = match_close(toks, j)
        j += 1
    return hi


def parse_members(toks: list[Tok], src: str, lo: int, hi: int) -> list[Member]:
    out: list[Member] = []
    i = lo
    while i < hi:
        t = toks[i]
        if t.p(";"):
            i += 1
            continue
        if t.p("@"):  # decorator
            i += 2
            if i < hi and toks[i].p("("):
                i = match_close(toks, i) + 1
            continue
        # modifiers
        while i < hi and toks[i].kind == "id" and toks[i].text in MODS and i + 1 < hi and not (
            toks[i + 1].p("(") or toks[i + 1].p("=") or toks[i + 1].p(":") or toks[i + 1].p(";")
        ):
            i += 1
        if i >= hi:
            break
        t = toks[i]
        if t.kind not in ("id", "str"):
            i += 1
            continue
        name, line = t.text, t.line
        j = i + 1
        if j < hi and (toks[j].p("?") or toks[j].p("!")):
            j += 1
        if j < hi and toks[j].p("<"):  # generics
            depth = 0
            while j < hi:
                if toks[j].p("<"):
                    depth += 1
                elif toks[j].p(">"):
                    depth -= 1
                    if depth == 0:
                        j += 1
                        break
                j += 1
        if j < hi and toks[j].p("("):
            pe = match_close(toks, j)
            k = pe + 1
            rl = rh = k
            if k < hi and toks[k].p(":"):
                rl = k + 1
                k = skip_type_until_body(toks, rl, hi)
                rh = k
            if k < hi and toks[k].p("{"):
                be = match_close(toks, k)
                out.append(Member(name, "method", line, j + 1, pe, rl, rh, k + 1, be))
                i = be + 1
                continue
            i = k + 1  # overload / abstract signature
            continue
        if j < hi and toks[j].p("="):
            k = j + 1
            if k < hi and toks[k].id("async"):
                k += 1
            if k < hi and toks[k].p("("):  # arrow function property
                pe = match_close(toks, k)
                k2 = pe + 1
                rl = rh = k2
                if k2 < hi and toks[k2].p(":"):
                    rl = k2 + 1
                    k2 = skip_type_until_body(toks, rl, hi)
                    rh = k2
                if k2 < hi and toks[k2].p("=>"):
                    b = k2 + 1
                    if b < hi and toks[b].p("{"):
                        be = match_close(toks, b)
                        out.append(Member(name, "arrow", line, k + 1, pe, rl, rh, b + 1, be))
                        i = be + 1
                    else:
                        e = b
                        depth = 0
                        while e < hi:
                            tt = toks[e]
                            if tt.kind == "punct":
                                if tt.text in "([{":
                                    depth += 1
                                elif tt.text in ")]}":
                                    depth -= 1
                                elif tt.text == ";" and depth == 0:
                                    break
                            e += 1
                        out.append(Member(name, "arrow", line, k + 1, pe, rl, rh, b, e))
                        i = e + 1
                    continue
            # plain property initialiser: skip to `;` at depth 0
            depth = 0
            while k < hi:
                tt = toks[k]
                if tt.kind == "punct":
                    if tt.text in "([{":
                        depth += 1
                    elif tt.text in ")]}":
                        depth -= 1
                    elif tt.text == ";" and depth == 0:
                        break
                k += 1
            i = k + 1
            continue
        # typed property declaration `x: T;` or anything else
        k = j
        depth = 0
        while k < hi:
            tt = toks[k]
            if tt.kind == "punct":
                if tt.text in "([{":
                    depth += 1
                elif tt.text in ")]}":
                    depth -= 1
                elif tt.text == ";" and depth == 0:
                    break
            k += 1
        i = k + 1
    return out


# ===================================================================== expression evaluation
def label_of(expr: str) -> str:
    e = expr.strip()
    m = re.fullmatch(r"[A-Za-z_$][\w$]*(?:\??\.[A-Za-z_$][\w$]*)*", e)
    if m:
        return re.split(r"\??\.", e)[-1]
    m = re.search(r"[A-Za-z_$][\w$]*", e)
    return ("expr:" + m.group(0)) if m else "expr"


@dataclass
class Ctx:
    root: Path
    toks: list
    src: str
    body_lo: int
    body_hi: int
    params: dict
    enums: dict
    notes: list = field(default_factory=list)


def eval_expr_src(expr: str, ctx_sub: Ctx | None, ctx: Ctx, depth: int):
    toks = lex(expr)
    alts = eval_alts(toks, expr, 0, len(toks), ctx, depth + 1, at=None, own=True)
    return alts


def eval_alts(toks, src, lo, hi, ctx: Ctx, depth=0, at=None, own=False):
    """-> list[str] of alternatives (with `{label}` placeholders) or None if not statically resolvable."""
    if depth > 6 or lo >= hi:
        return None
    # strip wrapping parens
    while hi - lo >= 2 and toks[lo].p("(") and match_close(toks, lo) == hi - 1:
        lo, hi = lo + 1, hi - 1
    # top-level ternary
    depth_p = 0
    q = c = None
    for k in range(lo, hi):
        t = toks[k]
        if t.kind == "punct":
            if t.text in "([{":
                depth_p += 1
            elif t.text in ")]}":
                depth_p -= 1
            elif t.text == "?" and depth_p == 0 and q is None:
                q = k
            elif t.text == ":" and depth_p == 0 and q is not None and c is None:
                c = k
    if q is not None and c is not None:
        a = eval_alts(toks, src, q + 1, c, ctx, depth + 1, at, own)
        b = eval_alts(toks, src, c + 1, hi, ctx, depth + 1, at, own)
        if a is None or b is None:
            return None
        return (a + b)[:CAP_ALTS]
    # top-level + concatenation
    parts = split_top(toks, lo, hi, "+")
    if len(parts) > 1:
        sets = []
        for (a, b) in parts:
            s = eval_alts(toks, src, a, b, ctx, depth + 1, at, own)
            if s is None:
                return None
            sets.append(s)
        return ["".join(p) for p in itertools.islice(itertools.product(*sets), CAP_ALTS)]
    # top-level || / ??  (fallback alternatives)
    for sep in ("||", "??"):
        parts = split_top(toks, lo, hi, sep)
        if len(parts) > 1:
            out = []
            for (a, b) in parts:
                s = eval_alts(toks, src, a, b, ctx, depth + 1, at, own)
                if s is None:
                    return None
                out += s
            return out[:CAP_ALTS]
    if hi - lo == 1:
        t = toks[lo]
        if t.kind == "str":
            return [t.text]
        if t.kind == "tpl":
            sets = []
            for kind, txt in t.parts or []:
                if kind == "s":
                    sets.append([txt])
                else:
                    sub = eval_expr_src(txt, None, ctx, depth)
                    # expand ternaries/literals inside `${...}`; anything else stays a `{label}` placeholder
                    if sub is not None and 1 <= len(sub) <= CAP_ALTS:
                        sets.append(sub)
                    else:
                        sets.append(["{" + label_of(txt) + "}"])
            return ["".join(p) for p in itertools.islice(itertools.product(*sets), CAP_ALTS)]
        if t.kind == "id" and not own:
            return lookup_var(toks, src, t.text, ctx, at, depth)
        if t.kind == "id" and own:
            return None
    # `this.x.y` member expressions with a known enum alias
    txt = src_of(src, toks, lo, hi)
    if txt in ctx.enums:
        return list(ctx.enums[txt])
    return None


def lookup_var(toks, src, name: str, ctx: Ctx, at: int | None, depth: int):
    """Find `const|let|var NAME [: T] = <expr>;` (and `NAME += ...`) in the enclosing member body before `at`."""
    lo, hi = ctx.body_lo, (at if at is not None else ctx.body_hi)
    found = None
    extra = False
    for k in range(lo, hi):
        t = toks[k]
        if t.kind == "id" and t.text in ("const", "let", "var") and k + 1 < hi and toks[k + 1].id(name):
            e = k + 2
            if e < hi and toks[e].p(":"):  # skip type annotation
                while e < hi and not toks[e].p("="):
                    e += 1
            if e < hi and toks[e].p("="):
                s = e + 1
                depth_p = 0
                z = s
                while z < hi:
                    tt = toks[z]
                    if tt.kind == "punct":
                        if tt.text in "([{":
                            depth_p += 1
                        elif tt.text in ")]}":
                            depth_p -= 1
                        elif tt.text == ";" and depth_p == 0:
                            break
                    z += 1
                found = eval_alts(toks, src, s, z, ctx, depth + 1, at=k)
        elif t.id(name) and k + 1 < hi and (toks[k + 1].p("+=") or toks[k + 1].p("=")) and not (k > 0 and toks[k - 1].kind == "id" and toks[k - 1].text in ("const", "let", "var")):
            extra = True
    if extra:
        ctx.notes.append(f"variable '{name}' reassigned/extended")
        return None
    if found is None and name in ctx.params:
        ctx.notes.append(f"path comes from parameter '{name}'")
    return found


# ===================================================================== call-site extraction
@dataclass
class Call:
    client: str
    file: str
    line: int
    cls: str
    extends: str
    member: str
    verb: str
    arg0_src: str
    paths: list
    resolution: str  # literal | derived | override | unresolved
    note: str
    inline_query: str
    params_keys: str
    params_src: str
    params_type: str
    body_src: str
    body_type: str
    response_type: str
    then_cast: str
    response_use: str
    catch_throws: str
    options: str
    base_expr: str
    request_method_override: str = ""


def parse_params(toks, src, lo, hi) -> dict:
    out = {}
    for (a, b) in split_top(toks, lo, hi, ","):
        if a >= b or toks[a].kind != "id":
            continue
        nm = toks[a].text
        k = a + 1
        if k < b and toks[k].p("?"):
            k += 1
        ty = ""
        if k < b and toks[k].p(":"):
            e = k + 1
            depth = 0
            while e < b:
                tt = toks[e]
                if tt.kind == "punct":
                    if tt.text in "([{<":
                        depth += 1
                    elif tt.text in ")]}>":
                        depth -= 1
                    elif tt.text == "=" and depth <= 0:
                        break
                e += 1
            ty = src_of(src, toks, k + 1, e)
        out[nm] = ty
    return out


def promise_inner(ret: str) -> str:
    ret = ret.strip()
    m = re.match(r"Promise<", ret)
    if not m:
        return ret
    depth, i = 0, len("Promise")
    for j in range(i, len(ret)):
        if ret[j] == "<":
            depth += 1
        elif ret[j] == ">":
            depth -= 1
            if depth == 0:
                return ret[i + 1 : j].strip()
    return ret[i + 1 :].strip()


def obj_keys(toks, src, lo, hi) -> tuple[list[str], list[str]]:
    """Keys of an object literal spanning toks[lo:hi] (including braces). -> (keys, spreads)"""
    keys, spreads = [], []
    if lo >= hi or not toks[lo].p("{"):
        return keys, spreads
    for (a, b) in split_top(toks, lo + 1, hi - 1, ","):
        if a >= b:
            continue
        t = toks[a]
        if t.p("..."):
            spreads.append(src_of(src, toks, a + 1, b))
        elif t.kind in ("id", "str"):
            keys.append(t.text)
    return keys, spreads


def find_key_value(toks, lo, hi, key: str):
    """In object literal toks[lo:hi]: return (vlo, vhi) token range of the value for `key`, or None."""
    if lo >= hi or not toks[lo].p("{"):
        return None
    for (a, b) in split_top(toks, lo + 1, hi - 1, ","):
        if a < b and toks[a].kind in ("id", "str") and toks[a].text == key:
            if a + 1 < b and toks[a + 1].p(":"):
                return (a + 2, b)
            return (a, a + 1)  # shorthand
    return None


def extract_calls(f: Path, root: Path, client: str, toks, src, classes: list[ClassDecl], service_names: set, enums) -> tuple[list[Call], list[dict]]:
    calls: list[Call] = []
    nonservice: list[dict] = []
    rel = str(f.relative_to(root))
    for cd in classes:
        is_service = cd.name in service_names
        for m in cd.members:
            lo, hi = m.body_lo, m.body_hi
            params = parse_params(toks, src, m.params_lo, m.params_hi)
            ret = promise_inner(src_of(src, toks, m.ret_lo, m.ret_hi)) if m.ret_hi > m.ret_lo else ""
            k = lo
            while k < hi - 3:
                if (
                    toks[k].id("this")
                    and toks[k + 1].p(".")
                    and toks[k + 2].kind == "id"
                    and toks[k + 2].text in VERBS
                    and toks[k + 3].p("(")
                ):
                    verb = toks[k + 2].text
                    pe = match_close(toks, k + 3)
                    if not is_service:
                        nonservice.append({"file": rel, "line": toks[k].line, "class": cd.name, "member": m.name})
                        k = pe
                        continue
                    args = split_top(toks, k + 4, pe, ",")
                    a0 = args[0] if args else (k + 4, k + 4)
                    arg0_src = src_of(src, toks, *a0)
                    ctx = Ctx(root, toks, src, lo, hi, params, enums)
                    request_method = ""
                    if verb == "request" and args:
                        kv = find_key_value(toks, a0[0], a0[1], "url")
                        mv = find_key_value(toks, a0[0], a0[1], "method")
                        if mv:
                            request_method = src_of(src, toks, *mv).strip("\"'`")
                        if kv:
                            a0 = kv
                            arg0_src = src_of(src, toks, *kv)
                    alts = eval_alts(toks, src, a0[0], a0[1], ctx, 0, at=k)
                    resolution = "unresolved"
                    note = "; ".join(ctx.notes)
                    if alts:
                        # literal if single str/tpl token without any placeholder-from-variable; else derived
                        single = a0[1] - a0[0] == 1 and toks[a0[0]].kind in ("str", "tpl")
                        resolution = "literal" if single else "derived"
                    ov = OVERRIDES.get((layout.canon(rel), f"{cd.name}.{m.name}", verb))
                    if ov and (not alts):
                        alts, resolution = list(ov["paths"]), "override"
                        note = ("EXTERNAL: " if ov.get("external") else "") + ov["why"]
                        OVERRIDES_USED.add((layout.canon(rel), f"{cd.name}.{m.name}", verb))
                    # chain
                    q = pe + 1
                    chain_names = []
                    while q + 2 < len(toks) and (toks[q].p(".") or toks[q].p("?.")) and toks[q + 1].kind == "id" and toks[q + 2].p("("):
                        ce2 = match_close(toks, q + 2)
                        chain_names.append((toks[q + 1].text, q + 3, ce2))
                        q = ce2 + 1
                    chain_src = {n: src_of(src, toks, a, b) for (n, a, b) in chain_names}
                    then_src = chain_src.get("then", "")
                    catch_src = chain_src.get("catch", "")
                    cast = ""
                    mc = re.search(r"\bas\s+([A-Za-z_][\w<>\[\]\.| ,{}:?]*)", then_src)
                    if mc:
                        cast = mc.group(1).strip()
                    use = "none"
                    if then_src:
                        if re.search(r"\.data\b", then_src):
                            use = "data"
                        elif re.search(r"\.status\b", then_src):
                            use = "status"
                        elif re.search(r"\.headers\b", then_src):
                            use = "headers"
                        else:
                            use = "other"
                    throws = "n/a"
                    if catch_src:
                        mt = re.search(r"throw\s+([^;}\n]+)", catch_src)
                        throws = mt.group(1).strip() if mt else "other"
                    # query / params / body / options
                    paramskeys, params_src, params_type = [], "", ""
                    for (a, b) in args[1:]:
                        v = find_key_value(toks, a, b, "params")
                        if v:
                            va, vb = v
                            if toks[va].p("{"):
                                ks, sp = obj_keys(toks, src, va, vb)
                                paramskeys += ks + [f"...{s}" for s in sp]
                                params_src = src_of(src, toks, va, vb)
                            else:
                                params_src = src_of(src, toks, va, vb)
                                base_id = params_src.strip()
                                if base_id in params:
                                    params_type = params[base_id]
                    body_src = body_type = ""
                    if verb in ("post", "put", "patch", "delete") and len(args) > 1:
                        body_src = src_of(src, toks, *args[1])
                        if body_src.strip() in params:
                            body_type = params[body_src.strip()]
                        elif toks[args[1][0]].p("{"):
                            ks, sp = obj_keys(toks, src, *args[1])
                            body_type = "{" + ",".join(ks + [f"...{s}" for s in sp]) + "}"
                    opts = []
                    for (a, b) in args[1:]:
                        for key in ("headers", "validateStatus", "responseType", "withCredentials"):
                            v = find_key_value(toks, a, b, key)
                            if v:
                                if key == "headers" and toks[v[0]].p("{"):
                                    hk, _ = obj_keys(toks, src, *v)
                                    opts.append("headers:" + "/".join(hk) if hk else "headers:{}")
                                else:
                                    opts.append(key)
                    # inline query
                    iq = ""
                    clean = []
                    for p in alts or []:
                        for (sf, lab), repl in PLACEHOLDER_SUBST.items():
                            if sf == layout.canon(rel) and "{" + lab + "}" in p:
                                p = p.replace("{" + lab + "}", repl)
                                resolution = "derived" if resolution == "literal" else resolution
                                note = (note + "; " if note else "") + f"{{{lab}}} substituted from client_overrides.PLACEHOLDER_SUBST"
                        if "?" in p:
                            base, _, qs = p.partition("?")
                            iq = iq or qs
                            clean.append(base)
                        else:
                            clean.append(p)
                    clean = list(dict.fromkeys(clean))  # drop duplicates that only differed by an inline ?query
                    base_expr = ""
                    for cc in classes:
                        if cc.name == cd.name and cc.super_base is not None:
                            base_expr = cc.super_base
                    calls.append(Call(
                        client=client, file=rel, line=toks[k].line, cls=cd.name, extends=cd.extends or "",
                        member=m.name, verb=(request_method.lower() or verb), arg0_src=arg0_src.strip(),
                        paths=clean, resolution=resolution, note=note, inline_query=iq,
                        params_keys=",".join(paramskeys), params_src=params_src, params_type=params_type,
                        body_src=body_src, body_type=body_type, response_type=ret, then_cast=cast,
                        response_use=use, catch_throws=throws, options=",".join(opts), base_expr=base_expr,
                        request_method_override=request_method,
                    ))
                    k = pe
                k += 1
    return calls, nonservice


# ===================================================================== path-literal references (kind B)
PATH_START = re.compile(r"^/(api|auth)/")


def strip_base(parts: list) -> tuple[list, str]:
    """If the template starts with a base-URL expression, drop it."""
    if parts and parts[0][0] == "e" and parts[0][1].strip() in BASE_EXPRS:
        return parts[1:], parts[0][1].strip()
    return parts, ""


def kind_for(toks: list[Tok], i: int, src: str) -> str:
    prev = toks[max(0, i - 6) : i]
    txt = " ".join(p.text for p in prev)
    if re.search(r"\bfetch\s*\($", txt) or (prev and prev[-1].p("(") and len(prev) > 1 and prev[-2].id("fetch")):
        return "fetch"
    if prev and prev[-1].p("{") and len(prev) > 1 and prev[-2].p("="):
        j = i - 3
        if j >= 0 and toks[j].id("action"):
            return "form-action"
        if j >= 0 and toks[j].id("href"):
            return "href"
        if j >= 0 and toks[j].id("src"):
            return "src"
    if len(prev) >= 2 and prev[-1].p("(") and prev[-2].id() and prev[-2].text in ("assign", "replace", "open"):
        return "redirect:" + prev[-2].text
    if len(prev) >= 2 and prev[-1].p("=") and prev[-2].id("href"):
        return "redirect:href="
    if len(prev) >= 2 and prev[-1].p("=") and prev[-2].id("action"):
        return "form-action(dynamic)"
    if len(prev) >= 2 and prev[-1].p("(") and prev[-2].id() and prev[-2].text in ("getFileURL", "getAssetUrl"):
        return "asset-url:" + prev[-2].text
    return "literal"


def extract_refs(f: Path, root: Path, client: str, toks, src, call_ranges: list[tuple[int, int]]) -> list[dict]:
    out = []
    rel = str(f.relative_to(root))
    for i, t in enumerate(toks):
        if t.kind not in ("str", "tpl"):
            continue
        if any(a <= t.pos < b for a, b in call_ranges):
            continue
        if t.kind == "str":
            if not PATH_START.match(t.text):
                continue
            text, base = t.text, ""
        else:
            parts, base = strip_base(t.parts or [])
            if not parts or parts[0][0] != "s" or not PATH_START.match(parts[0][1]):
                continue
            ctx = Ctx(root, toks, src, 0, 0, {}, {})
            sets = []
            for kind, txt in parts:
                if kind == "s":
                    sets.append([txt])
                else:
                    sub = eval_expr_src(txt, None, ctx, 0)
                    if sub is not None and 1 <= len(sub) <= CAP_ALTS:
                        sets.append(sub)
                    else:
                        sets.append(["{" + label_of(txt) + "}"])
            alts = ["".join(p) for p in itertools.islice(itertools.product(*sets), CAP_ALTS)]
            text = alts[0]
            out_alts = alts
        kind = kind_for(toks, i, src)
        alts_final = out_alts if t.kind == "tpl" else [text]
        out.append({
            "client": client, "file": rel, "line": t.line, "kind": kind,
            "paths": [a.partition("?")[0] for a in alts_final],
            "base_expr": base, "raw": src[t.pos : t.end][:160], "pos": t.pos,
        })
    return out


# ===================================================================== usage counts for service methods
def app_of(rel: str) -> str:
    parts = rel.split("/")
    if parts[0] == "apps":
        return parts[1]
    if parts[0] == "packages":
        return "pkg-" + parts[1]
    return parts[0]


def usage_counts(root: Path, all_files: list[tuple[Path, str]], calls: list[Call], class_ext: dict) -> dict:
    """(file, class, member) -> {app: occurrences of `.member` in files that mention the class or a subclass}.

    Heuristic liveness hint [inferred]: collisions can only over-count (a generic name such as `retrieve`
    appearing elsewhere), never under-count, except for destructured/aliased use. The defining file counts
    `this.member` self-calls too (but not the method definition itself, which has no leading dot).
    """
    texts = {f: f.read_text(encoding="utf-8") for f, _ in all_files}
    subs: dict[str, set] = {}
    for c in {c.cls for c in calls}:
        names = {c}
        grew = True
        while grew:
            grew = False
            for k, b in class_ext.items():
                if b in names and k not in names:
                    names.add(k)
                    grew = True
        subs[c] = names
    mention_cache: dict[tuple, list[Path]] = {}
    uses: dict[tuple, dict] = {}
    evidence_by_key: dict[tuple, str] = {}
    class_consumers: dict[tuple, dict] = {}
    recv_rx = re.compile(r"\b\w*[sS]ervice\w*\??\s*\.\s*([A-Za-z_]\w*)")
    loose: dict[str, dict[str, int]] = {}
    for f, t in texts.items():
        a = app_of(str(f.relative_to(root)))
        for m in recv_rx.finditer(t):
            d = loose.setdefault(a, {})
            d[m.group(1)] = d.get(m.group(1), 0) + 1
    for c in calls:
        key = (c.file, c.cls, c.member)
        if key in uses:
            continue
        deff = root / c.file
        ck = (c.file, c.cls)
        if ck not in mention_cache:
            name_pat = re.compile(r"\b(?:" + "|".join(re.escape(n) for n in sorted(subs[c.cls])) + r")\b")
            # apps/web services are mostly consumed as default-exported singletons: `import svc from "@/services/x.service"`
            stem_pat = re.compile(r"""from\s+["'][^"']*[/.]""" + re.escape(deff.stem) + r"""["']""")
            in_pkg_services = c.file.startswith("packages/services/")
            own_dir = c.file.split("/")[0] + "/" + c.file.split("/")[1]
            cands = []
            for f, t in texts.items():
                if f == deff:
                    continue
                rel = str(f.relative_to(root))
                if stem_pat.search(t):
                    cands.append(f)
                elif name_pat.search(t):
                    # a class-name mention only counts where that class can actually be the one imported
                    if in_pkg_services and ("@plane/services" in t or rel.startswith("packages/services/")):
                        cands.append(f)
                    elif not in_pkg_services and rel.startswith(own_dir + "/"):
                        cands.append(f)
            mention_cache[ck] = cands
        mem = re.compile(r"\.\s*" + re.escape(c.member) + r"\b")
        per_app: dict[str, int] = {}
        for f in mention_cache[ck]:
            n = len(mem.findall(texts[f]))
            if n:
                a = app_of(str(f.relative_to(root)))
                per_app[a] = per_app.get(a, 0) + n
        # self-calls in the defining file (counted under the defining app)
        if deff in texts:
            n = len(mem.findall(texts[deff]))
            if n:
                per_app[app_of(c.file)] = per_app.get(app_of(c.file), 0) + n
        evidence = "strict" if per_app else "none"
        class_apps = {}
        for f in mention_cache[ck]:
            a = app_of(str(f.relative_to(root)))
            class_apps[a] = class_apps.get(a, 0) + 1
        class_consumers[key] = class_apps
        if not per_app:
            # loose fallback: any `<something>Service.<member>` in the apps (covers property inheritance such as
            # `this.cycleService` declared in a base store class, where the class name is never mentioned)
            total = sum(members.get(c.member, 0) for members in loose.values())
            if total:
                per_app = {"?": total}  # used somewhere, app not attributable by name alone
                evidence = "loose"
        uses[key] = per_app
        evidence_by_key[key] = evidence
    return uses, evidence_by_key, class_consumers


# ===================================================================== driver
def scan(root: Path) -> dict:
    enums = {}
    for alias, enum in THIS_ENUM_ALIASES.items():
        vals = resolve_enum(root, enum)
        if vals:
            enums[alias] = vals
    files: list[tuple[Path, str, bool]] = []
    seen = set()
    for rel, label, requested in DEFAULT_CLIENTS:
        for f in iter_sources(root, rel):
            if f in seen:
                continue
            seen.add(f)
            files.append((f, label, requested))
    parsed = {}
    class_ext: dict[str, str] = {}
    for f, label, _ in files:
        src = f.read_text(encoding="utf-8")
        toks = lex(src)
        classes = find_classes(toks, src)
        parsed[f] = (src, toks, classes)
        for cd in classes:
            if cd.extends:
                class_ext.setdefault(cd.name, cd.extends)
    service_names: set[str] = set()
    changed = True
    while changed:
        changed = False
        for name, base in class_ext.items():
            if name not in service_names and (base == "APIService" or base in service_names):
                service_names.add(name)
                changed = True
    all_calls: list[Call] = []
    nonservice: list[dict] = []
    refs: list[dict] = []
    textual_total = 0
    per_client_textual: dict[str, int] = {}
    verb_re = re.compile(r"this\.(get|post|put|patch|delete|request)\(")
    for f, label, requested in files:
        src, toks, classes = parsed[f]
        member_ranges: list[tuple[int, int]] = []
        calls, ns = extract_calls(f, root, label, toks, src, classes, service_names, enums)
        all_calls += calls
        nonservice += [dict(x, client=label) for x in ns]
        # argument lists of `this.verb(...)` calls are excluded from the literal-reference scan (no double listing)
        ranges = []
        called_members = {(c.cls, c.member) for c in calls}
        for cd in classes:
            for mem in cd.members:
                if (cd.name, mem.name) in called_members and mem.body_hi > mem.body_lo:
                    # literals inside a member that already yields a call are the call's own path pieces
                    member_ranges.append((toks[mem.body_lo].pos, toks[mem.body_hi].pos))
        for k in range(len(toks) - 3):
            if toks[k].id("this") and toks[k + 1].p(".") and toks[k + 2].kind == "id" and toks[k + 2].text in VERBS and toks[k + 3].p("("):
                pe = match_close(toks, k + 3)
                ranges.append((toks[k + 3].end, toks[pe].pos))
        refs += [r for r in extract_refs(f, root, label, toks, src, ranges)
                 if not (r["kind"] == "literal" and any(a <= r["pos"] < b for a, b in member_ranges))]
        # independent textual count (regex over raw text, comments crudely removed)
        n = 0
        for line in src.splitlines():
            s = line.strip()
            if s.startswith("//") or s.startswith("*") or s.startswith("/*"):
                continue
            n += len(verb_re.findall(line))
        textual_total += n
        per_client_textual[label] = per_client_textual.get(label, 0) + n
    uses, use_evidence, class_consumers = usage_counts(root, [(f, l) for f, l, _ in files], all_calls, class_ext)
    return {
        "calls": all_calls, "refs": refs, "nonservice": nonservice, "uses": uses, "use_evidence": use_evidence,
        "class_consumers": class_consumers,
        "textual_total": textual_total, "per_client_textual": per_client_textual,
        "service_classes": sorted(service_names), "enums": enums, "n_files": len(files),
        "requested": {label: req for _, label, req in DEFAULT_CLIENTS},
    }


def call_to_dict(c: Call, uses: dict, evidence: dict, class_consumers: dict) -> dict:
    d = c.__dict__.copy()
    d["paths"] = list(c.paths)
    per_app = uses.get((c.file, c.cls, c.member), {})
    d["method_uses_by_app"] = per_app
    d["class_consumer_apps"] = class_consumers.get((c.file, c.cls, c.member), {})
    d["method_use_evidence"] = evidence.get((c.file, c.cls, c.member), "none")
    d["method_uses_elsewhere"] = sum(per_app.values())
    return d


def write_outputs(res: dict, out: Path):
    out.mkdir(parents=True, exist_ok=True)
    calls = [call_to_dict(c, res["uses"], res["use_evidence"], res["class_consumers"]) for c in res["calls"]]
    (out / "client-calls.json").write_text(json.dumps(calls, indent=1, sort_keys=True) + "\n")
    (out / "client-refs.json").write_text(json.dumps(res["refs"], indent=1, sort_keys=True) + "\n")
    cols = ["client", "file", "line", "cls", "member", "verb", "paths", "resolution", "inline_query", "params_keys", "params_type",
            "body_type", "response_type", "then_cast", "response_use", "catch_throws", "options", "method_uses_elsewhere", "method_uses_by_app", "method_use_evidence", "note"]
    with open(out / "client-calls.csv", "w", newline="") as fh:
        wr = csv.writer(fh)
        wr.writerow(cols)
        for c in sorted(calls, key=lambda c: (c["client"], c["file"], c["line"])):
            wr.writerow([re.sub(r"\s+", " ", v) if isinstance(v, str) else v for v in
                         (";".join(c["paths"]) if k == "paths" else (json.dumps(c[k], sort_keys=True) if isinstance(c[k], dict) else c[k]) for k in cols)])
    with open(out / "client-refs.csv", "w", newline="") as fh:
        wr = csv.writer(fh)
        wr.writerow(["client", "file", "line", "kind", "paths", "base_expr"])
        for r in sorted(res["refs"], key=lambda r: (r["client"], r["file"], r["line"])):
            wr.writerow([r["client"], r["file"], r["line"], r["kind"], ";".join(r["paths"]), r["base_expr"]])
    unresolved = [c for c in calls if c["resolution"] == "unresolved"]
    (out / "client-unresolved.json").write_text(json.dumps(unresolved, indent=1, sort_keys=True) + "\n")
    stats = {
        "files_scanned": res["n_files"],
        "service_classes": len(res["service_classes"]),
        "this_verb_textual_occurrences": res["textual_total"],
        "this_verb_textual_by_client": res["per_client_textual"],
        "calls_extracted": len(calls),
        "calls_by_client": {k: sum(1 for c in calls if c["client"] == k) for k in sorted({c["client"] for c in calls})},
        "calls_in_non_service_classes_skipped": len(res["nonservice"]),
        "resolution": {k: sum(1 for c in calls if c["resolution"] == k) for k in ("literal", "derived", "override", "unresolved")},
        "path_refs_non_axios": len(res["refs"]),
        "path_refs_by_kind": {k: sum(1 for r in res["refs"] if r["kind"].split(":")[0] == k) for k in sorted({r["kind"].split(":")[0] for r in res["refs"]})},
        "enums_used": res["enums"],
        "calls_with_no_use_elsewhere": sum(1 for c in calls if c["method_uses_elsewhere"] == 0),
        "method_use_evidence": {k: sum(1 for c in calls if c["method_use_evidence"] == k) for k in ("strict", "loose", "none")},
        "unresolved_list": [f"{c['file']}:{c['line']} {c['cls']}.{c['member']}" for c in unresolved],
        "override_keys_unused": sorted(" | ".join(k) for k in set(OVERRIDES) - OVERRIDES_USED),
    }
    stats["recall_check_extracted_plus_nonservice_over_textual"] = (
        f"{len(calls) + len(res['nonservice'])}/{res['textual_total']}"
    )
    (out / "client-scan-stats.json").write_text(json.dumps(stats, indent=1, sort_keys=True) + "\n")
    return stats


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    a = ap.parse_args()
    res = scan(a.root.resolve())
    stats = write_outputs(res, a.out)
    print(json.dumps(stats, indent=1, sort_keys=True))


if __name__ == "__main__":
    main()
