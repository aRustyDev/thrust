# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Static (ast-only) index of the Django source tree: modules, classes, imports.

Used by server_routes.py to resolve `from plane.app.views import X` chains to the real
`class X` definition (file:line), to walk class bases, and to read handler methods.
No Django import, no execution of project code. Stdlib only.
"""

from __future__ import annotations

import ast
from dataclasses import dataclass, field
from pathlib import Path

DRF_VIEWSET_DEFAULTS = {
    # DRF base classes whose action methods are inherited when the project does not override them.
    "ModelViewSet": ["list", "create", "retrieve", "update", "partial_update", "destroy"],
    "ReadOnlyModelViewSet": ["list", "retrieve"],
    "GenericViewSet": [],
    "ViewSet": [],
}
DRF_GENERIC_VIEW_METHODS = {
    # concrete generic views -> HTTP methods they define
    "ListAPIView": ["get"],
    "CreateAPIView": ["post"],
    "RetrieveAPIView": ["get"],
    "UpdateAPIView": ["put", "patch"],
    "DestroyAPIView": ["delete"],
    "ListCreateAPIView": ["get", "post"],
    "RetrieveUpdateAPIView": ["get", "put", "patch"],
    "RetrieveDestroyAPIView": ["get", "delete"],
    "RetrieveUpdateDestroyAPIView": ["get", "put", "patch", "delete"],
}
HTTP_METHODS = ("get", "post", "put", "patch", "delete")


@dataclass
class ClassInfo:
    name: str
    file: Path
    lineno: int
    node: ast.ClassDef
    module: str
    bases: list = field(default_factory=list)  # list[ClassInfo | str]  (str = unresolved/external name)
    methods: dict = field(default_factory=dict)  # name -> ast.FunctionDef

    def loc(self, root: Path) -> str:
        return f"{self.file.relative_to(root)}:{self.lineno}"


class PyIndex:
    def __init__(self, root: Path):
        self.root = root  # Plane tree root
        self.pyroot = root / "apps" / "api"  # python import root (contains `plane/`)
        self._trees: dict[Path, ast.Module] = {}
        self._classes: dict[tuple[Path, str], ClassInfo] = {}

    # ---- module <-> file -------------------------------------------------------------
    def module_file(self, modname: str) -> Path | None:
        base = self.pyroot.joinpath(*modname.split("."))
        if (base / "__init__.py").exists():
            return base / "__init__.py"
        if base.with_suffix(".py").exists():
            return base.with_suffix(".py")
        return None

    def file_module(self, f: Path) -> str:
        rel = f.relative_to(self.pyroot).with_suffix("")
        parts = list(rel.parts)
        if parts[-1] == "__init__":
            parts = parts[:-1]
        return ".".join(parts)

    def tree(self, f: Path) -> ast.Module:
        if f not in self._trees:
            self._trees[f] = ast.parse(f.read_text(encoding="utf-8"), filename=str(f))
        return self._trees[f]

    def source(self, f: Path) -> str:
        return f.read_text(encoding="utf-8")

    # ---- imports ---------------------------------------------------------------------
    def abs_module(self, f: Path, level: int, module: str | None) -> str:
        if level == 0:
            return module or ""
        pkg = self.file_module(f).split(".")
        if f.name != "__init__.py":
            pkg = pkg[:-1]
        pkg = pkg[: len(pkg) - (level - 1)] if level > 1 else pkg
        return ".".join(pkg + ([module] if module else []))

    def import_table(self, f: Path) -> dict[str, tuple[str, str | None]]:
        """local name -> (module, attr-or-None). attr None means `import module [as x]`."""
        out: dict[str, tuple[str, str | None]] = {}
        for node in self.tree(f).body:
            self._collect_imports(node, f, out)
        return out

    def _collect_imports(self, node, f, out):
        if isinstance(node, ast.ImportFrom):
            mod = self.abs_module(f, node.level, node.module)
            for a in node.names:
                out[a.asname or a.name] = (mod, a.name)
        elif isinstance(node, ast.Import):
            for a in node.names:
                out[a.asname or a.name.split(".")[0]] = (a.name, None)
        elif isinstance(node, (ast.If, ast.Try)):
            for sub in ast.walk(node):
                if isinstance(sub, (ast.ImportFrom, ast.Import)) and sub is not node:
                    self._collect_imports(sub, f, out)

    # ---- symbol resolution -----------------------------------------------------------
    def resolve_symbol(self, modname: str, name: str, _seen=None):
        """Return ('class', ClassInfo) | ('func', file, FunctionDef) | ('var', file, node) | None."""
        _seen = _seen or set()
        if (modname, name) in _seen:
            return None
        _seen.add((modname, name))
        f = self.module_file(modname)
        if f is None:
            return None
        for node in self.tree(f).body:
            if isinstance(node, ast.ClassDef) and node.name == name:
                return ("class", self.class_info(f, node))
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == name:
                return ("func", f, node)
            if isinstance(node, ast.Assign):
                for t in node.targets:
                    if isinstance(t, ast.Name) and t.id == name:
                        return ("var", f, node)
        imp = self.import_table(f)
        if name in imp:
            mod, attr = imp[name]
            if attr is not None:
                r = self.resolve_symbol(mod, attr, _seen)
                if r:
                    return r
                # `from pkg import submodule`
                sub = f"{mod}.{attr}"
                if self.module_file(sub):
                    return ("module", sub)
        # star imports
        for node in self.tree(f).body:
            if isinstance(node, ast.ImportFrom) and any(a.name == "*" for a in node.names):
                r = self.resolve_symbol(self.abs_module(f, node.level, node.module), name, _seen)
                if r:
                    return r
        return None

    def resolve_name_in_file(self, f: Path, name: str):
        """Resolve a bare name used inside file f (local def first, then imports)."""
        mod = self.file_module(f)
        r = None
        for node in self.tree(f).body:
            if isinstance(node, ast.ClassDef) and node.name == name:
                return ("class", self.class_info(f, node))
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == name:
                return ("func", f, node)
        imp = self.import_table(f)
        if name in imp:
            m, attr = imp[name]
            if attr is None:
                return ("module", m)
            r = self.resolve_symbol(m, attr)
            if r is None and self.module_file(f"{m}.{attr}"):
                r = ("module", f"{m}.{attr}")
            return r
        return self.resolve_symbol(mod, name)

    # ---- classes ---------------------------------------------------------------------
    def class_info(self, f: Path, node: ast.ClassDef) -> ClassInfo:
        key = (f, node.name)
        if key in self._classes:
            return self._classes[key]
        ci = ClassInfo(
            name=node.name,
            file=f,
            lineno=node.lineno,
            node=node,
            module=self.file_module(f),
        )
        self._classes[key] = ci
        for st in node.body:
            if isinstance(st, (ast.FunctionDef, ast.AsyncFunctionDef)):
                ci.methods[st.name] = st
        for b in node.bases:
            ci.bases.append(self._resolve_base(f, b))
        return ci

    def _resolve_base(self, f: Path, b: ast.expr):
        if isinstance(b, ast.Name):
            r = self.resolve_name_in_file(f, b.id)
            if r and r[0] == "class":
                return r[1]
            return b.id
        if isinstance(b, ast.Attribute):
            return ast.unparse(b).split(".")[-1]
        return ast.unparse(b)

    def mro(self, ci: ClassInfo):
        """Depth-first, left-to-right approximation of the MRO (adequate for these hierarchies)."""
        seen, out = set(), []

        def walk(c):
            if isinstance(c, str):
                out.append(c)
                return
            if id(c) in seen:
                return
            seen.add(id(c))
            out.append(c)
            for b in c.bases:
                walk(b)

        walk(ci)
        return out

    def find_method(self, ci: ClassInfo, name: str):
        """Return (ClassInfo, FunctionDef) for the first class in the MRO defining `name`."""
        for c in self.mro(ci):
            if isinstance(c, ClassInfo) and name in c.methods:
                return c, c.methods[name]
        return None

    def external_bases(self, ci: ClassInfo) -> list[str]:
        return [c for c in self.mro(ci) if isinstance(c, str)]

    def class_attr(self, ci: ClassInfo, attr: str):
        """First class-level `attr = <expr>` in the MRO -> (ClassInfo, ast expr)."""
        for c in self.mro(ci):
            if not isinstance(c, ClassInfo):
                continue
            for st in c.node.body:
                if isinstance(st, ast.Assign):
                    for t in st.targets:
                        if isinstance(t, ast.Name) and t.id == attr:
                            return c, st.value
        return None
