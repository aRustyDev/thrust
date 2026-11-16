#!/bin/sh
# ADR-0009's layout guard (`.agents/plans/contrail/adrs/architecture/0009-crate-directory-layout.md`, crates-layout PR 4).
#
#   scripts/check-layout.sh               this repository: the tracked files (git ls-files) and the workspace (cargo metadata)
#   scripts/check-layout.sh --self-test   the controls: a small world, accepted, then broken one way at a time, each refused
#
# 1. Every workspace member is a directory under crates/.
# 2. A directory under crates/ is a crate (a workspace member) or a group, never both (ADR-0009 rule 1). A group holds crates or
#    further groups and no file but its README.md, and that README.md names each directory in the group as `<name>/`. No crate
#    contains another. Inside a crate, anything goes.
# 3. No crate has a normal or build dependency on a crate under crates/testing/ or crates/spikes/ (rules 5 and 6). A
#    dev-dependency is how a test uses a fixture, so it is allowed.
# 4. Nothing under crates/core/ has a normal or build dependency on a crate outside crates/core/ (rule 4). Dev-dependencies are
#    allowed: thrust-core's tests run against the memory adapters, which ship in neither core's graph nor its build.
# 5. What names a path that a move can break, and says nothing when it does, names one that exists: every literal path in a
#    workflow's `paths:` or `paths-ignore:` filter is a tracked file, and a glob's directory holds one (a stale filter stops the
#    workflow from running); every exclude_globs and examine_globs entry of .cargo/mutants.toml (a stale exclusion starts
#    mutating what it excluded); every .gitignore line under crates/ lies inside a crate (a stale one lets build output show).
#
# Package names are not checked: ADR-0009 keeps them as they are, and the directory may differ from the name.
#
# Exit: 0 clean; 1 a rule broken (each on its own line); 2 nothing to check (no workspace member, or no tracked file under
# crates/): a check over nothing passes vacuously, which is not evidence.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cat >"$TMP/layout.py" <<'PY'
import json
import os
import re
import shutil
import sys
import tempfile
import tomllib

GLOB = re.compile(r"[*?\[]")


def check(root, files, meta):
    """The rules over one world. Returns (exit code, the lines to print, how many of each thing it checked)."""
    tracked = set(files)
    seen = {"groups": 0, "workflow filter entries": 0, "mutants.toml globs": 0, ".gitignore lines under crates/": 0}
    wr = meta.get("workspace_root", "").rstrip("/") + "/"
    ids = set(meta.get("workspace_members", []))
    members = {}
    out = []
    for p in meta.get("packages", []):
        if p["id"] not in ids:
            continue
        mp = p["manifest_path"]
        if not mp.startswith(wr):
            out.append(f"{p['name']}: its manifest {mp} is outside the workspace root")
            continue
        members[mp[len(wr):-len("/Cargo.toml")]] = p
    under = [f for f in files if f.startswith("crates/")]
    if not members or not under:
        return 2, ["ERROR: no workspace member, or no tracked file under crates/: there is nothing to check"], seen
    name = {d: p["name"] for d, p in members.items()}

    def owner(path):
        best = None
        for d in members:
            if path.startswith(d + "/") and (best is None or len(d) > len(best)):
                best = d
        return best

    # 1 and the crate half of 2
    for d in sorted(members):
        if not d.startswith("crates/"):
            out.append(f"{name[d]} is at {d}/, outside crates/ (rule 1)")
        outer = owner(d)
        if outer:
            out.append(f"{name[d]} at {d}/ is inside the crate {name[outer]} at {outer}/: no crate contains another (rule 2)")

    # the group half of 2: every directory of a tracked file under crates/ that no crate holds
    dirs = set()
    for f in under:
        parts = f.split("/")
        for i in range(1, len(parts)):
            dirs.add("/".join(parts[:i]))
    for d in sorted(dirs):
        if d in members or owner(d + "/"):
            continue
        if not any(m.startswith(d + "/") for m in members):
            out.append(f"{d}/ is neither a crate nor a group: no workspace member is in it (rule 2; a crate must be a member)")
            continue
        seen["groups"] += 1
        readme = f"{d}/README.md"
        if readme not in tracked:
            out.append(f"{d}/ is a group with no README.md (rule 2)")
            text = ""
        else:
            with open(os.path.join(root, readme), encoding="utf-8") as fh:
                text = fh.read()
        for f in sorted(f for f in under if os.path.dirname(f) == d and os.path.basename(f) != "README.md"):
            out.append(f"{f}: a group holds no file but its README.md; a directory is a crate or a group, never both (rule 2)")
        for c in sorted(x for x in dirs if os.path.dirname(x) == d):
            if text and f"`{os.path.basename(c)}/`" not in text:
                out.append(f"{readme} does not name `{os.path.basename(c)}/`, which is in the group (rule 2)")

    # 3 and 4: normal and build edges between members
    for d in sorted(members):
        for dep in members[d].get("dependencies", []):
            if "path" not in dep or dep.get("kind") == "dev":
                continue
            to = dep["path"][len(wr):] if dep["path"].startswith(wr) else dep["path"]
            kind = dep.get("kind") or "normal"
            if to.startswith(("crates/testing/", "crates/spikes/")):
                out.append(f"{name[d]} has a {kind} dependency on {dep['name']} at {to}/: testing/ and spikes/ never ship (rule 3)")
            if d.startswith("crates/core/") and not to.startswith("crates/core/"):
                out.append(f"{name[d]} in core/ has a {kind} dependency on {dep['name']} at {to}/, outside core/ (rule 4)")

    # 5
    def exists(entry, what):
        e = entry.strip().strip("'\"").lstrip("!")
        if e.startswith("/"):
            e = e[1:]
        m = GLOB.search(e)
        if not m:
            if e.endswith("/"):
                if not any(f.startswith(e) for f in tracked):
                    out.append(f"{what}: {entry} names no tracked file (rule 5)")
            elif e not in tracked:
                out.append(f"{what}: {entry} names no tracked file (rule 5)")
            return
        prefix = e[:m.start()]
        if "/" not in prefix:
            return  # a pattern for any directory (**/x, *.rs): nothing to look for
        base = prefix.rsplit("/", 1)[0] + "/"
        if not any(f.startswith(base) for f in tracked):
            out.append(f"{what}: {entry}: no tracked file is under {base} (rule 5)")

    for wf in sorted(f for f in files if re.match(r"^\.github/workflows/[^/]+\.ya?ml$", f)):
        with open(os.path.join(root, wf), encoding="utf-8") as fh:
            lines = fh.read().split("\n")
        i = 0
        while i < len(lines):
            m = re.match(r"^(\s*)(paths|paths-ignore):\s*(#.*)?$", lines[i])
            i += 1
            if not m:
                continue
            ind = len(m.group(1))
            while i < len(lines):
                item = re.match(r"^(\s*)-\s+(.*?)\s*(#.*)?$", lines[i])
                if item and len(item.group(1)) > ind:
                    exists(item.group(2), f"{wf} `{m.group(2)}:`")
                    seen["workflow filter entries"] += 1
                    i += 1
                elif lines[i].strip() == "" or lines[i].lstrip().startswith("#"):
                    i += 1
                else:
                    break

    mt = ".cargo/mutants.toml"
    if mt in tracked:
        with open(os.path.join(root, mt), "rb") as fh:
            conf = tomllib.load(fh)
        for key in ("exclude_globs", "examine_globs"):
            for g in conf.get(key, []):
                exists(g, f"{mt} {key}")
                seen["mutants.toml globs"] += 1

    if ".gitignore" in tracked:
        with open(os.path.join(root, ".gitignore"), encoding="utf-8") as fh:
            for n, line in enumerate(fh, 1):
                e = line.strip()
                if e.startswith("/"):
                    e = e[1:]
                if not e.startswith("crates/"):
                    continue
                seen[".gitignore lines under crates/"] += 1
                m = GLOB.search(e)
                lit = e[:m.start()] if m else e.rstrip("/")
                if not owner(lit + "/"):
                    out.append(f".gitignore:{n}: {line.strip()} lies inside no crate (rule 5)")

    return (1 if out else 0), out, seen


def git_world(root, files_path, meta_path):
    with open(files_path, encoding="utf-8") as fh:
        files = [l for l in fh.read().split("\n") if l]
    with open(meta_path, encoding="utf-8") as fh:
        meta = json.load(fh)
    code, lines, seen = check(root, files, meta)
    for l in lines:
        print(("FAIL: " if code == 1 else "") + l)
    if code == 0:
        print(f"ok: {len(meta['workspace_members'])} crates, every directory under crates/ a crate or a named group, the "
              "dependency rules and the named paths hold. Checked: "
              + ", ".join(f"{n} {what}" for what, n in seen.items()))
    return code


# ---- --self-test ----------------------------------------------------------------------------------------------------------------
def base_world():
    """A consistent world: files (path -> text) and crates (dir -> [(dep dir, kind)])."""
    files = {
        "crates/README.md": "`core/` `adapters/` `testing/` `spikes/` `server/`",
        "crates/core/README.md": "`types/` `usecases/`",
        "crates/core/types/Cargo.toml": "", "crates/core/types/src/lib.rs": "",
        "crates/core/usecases/Cargo.toml": "", "crates/core/usecases/src/lib.rs": "",
        "crates/adapters/README.md": "`memory/`",
        "crates/adapters/memory/Cargo.toml": "", "crates/adapters/memory/src/lib.rs": "",
        "crates/testing/README.md": "`fixtures/`",
        "crates/testing/fixtures/README.md": "`seed/`",
        "crates/testing/fixtures/seed/Cargo.toml": "", "crates/testing/fixtures/seed/src/lib.rs": "",
        "crates/spikes/README.md": "`probe/`",
        "crates/spikes/probe/Cargo.toml": "", "crates/spikes/probe/src/lib.rs": "",
        "crates/server/Cargo.toml": "", "crates/server/src/main.rs": "", "crates/server/tests/t.rs": "",
        ".github/workflows/deploy.yml": "on:\n  push:\n    paths:\n      - crates/server/Cargo.toml\n      # a comment\n"
                                        "      - '.github/actions/x/**'\n  pull_request:\njobs: {}\n",
        ".github/actions/x/action.yml": "",
        ".cargo/mutants.toml": 'exclude_globs = ["crates/testing/**", "**/common/**", "*.md"]\n',
        ".gitignore": "target/\ncrates/server/build/\ncrates/server/.wrangler.*.toml\n",
    }
    crates = {
        "crates/core/types": [],
        "crates/core/usecases": [("crates/core/types", None), ("crates/adapters/memory", "dev")],
        "crates/adapters/memory": [("crates/core/types", None)],
        "crates/testing/fixtures/seed": [("crates/core/types", None)],
        "crates/spikes/probe": [("crates/core/types", None)],
        "crates/server": [("crates/core/usecases", None), ("crates/adapters/memory", None),
                          ("crates/testing/fixtures/seed", "dev")],
    }
    return files, crates


def run_world(files, crates):
    d = tempfile.mkdtemp()
    try:
        for p, text in files.items():
            os.makedirs(os.path.join(d, os.path.dirname(p)), exist_ok=True)
            with open(os.path.join(d, p), "w", encoding="utf-8") as fh:
                fh.write(text)
        pkgs = [{"name": "thrust-" + c.rsplit("/", 1)[1], "id": f"path+file://{d}/{c}#0.1.0",
                 "manifest_path": f"{d}/{c}/Cargo.toml",
                 "dependencies": [{"name": "thrust-" + t.rsplit("/", 1)[1], "path": f"{d}/{t}", "kind": k} for t, k in deps]
                 + [{"name": "serde", "kind": None}]}
                for c, deps in crates.items()]
        meta = {"workspace_root": d, "workspace_members": [p["id"] for p in pkgs], "packages": pkgs}
        code, lines, _ = check(d, sorted(files), meta)
        return code, lines
    finally:
        shutil.rmtree(d)


def self_test():
    cases = []

    def case(label, want, change=None, expect=""):
        """want: the exit code; expect: a phrase one of the printed lines must contain, so the case proves the rule it
        names fired and not some other one."""
        files, crates = base_world()
        if change:
            change(files, crates)
        cases.append((label, want, expect, files, crates))

    def move_crate(old, new):
        def f(files, crates):
            for p in [p for p in files if p.startswith(old + "/")]:
                files[new + p[len(old):]] = files.pop(p)
            crates[new] = crates.pop(old)
            for c in crates:
                crates[c] = [(new if t == old else t, k) for t, k in crates[c]]
        return f

    def set_dep(frm, to, kind):
        def f(files, crates):
            crates[frm] = [x for x in crates[frm] if x[0] != to] + [(to, kind)]
        return f

    def set_file(path, text):
        def f(files, crates):
            files[path] = text
        return f

    def drop_file(path):
        def f(files, crates):
            del files[path]
        return f

    case("a consistent world (dev-dependencies on testing/ and from core/ outside it included) is accepted", 0)
    case("a crate outside crates/ is refused", 1, move_crate("crates/server", "tools/server"), "outside crates/ (rule 1)")
    case("a crate inside another crate is refused", 1,
         lambda f, c: (f.update({"crates/server/inner/Cargo.toml": "", "crates/server/inner/src/lib.rs": ""}),
                       c.update({"crates/server/inner": []})), "no crate contains another")
    case("a directory under crates/ with no crate in it is refused", 1, set_file("crates/stray/notes.txt", ""),
         "crates/stray/ is neither a crate nor a group")
    case("a crate that is not a workspace member is refused", 1,
         lambda f, c: f.update({"crates/adapters/extra/Cargo.toml": "", "crates/adapters/extra/src/lib.rs": ""}),
         "crates/adapters/extra/ is neither a crate nor a group")
    case("a group with no README.md is refused", 1, drop_file("crates/adapters/README.md"), "crates/adapters/ is a group with no README.md")
    case("the top-level README.md missing is refused", 1, drop_file("crates/README.md"), "crates/ is a group with no README.md")
    case("a group README.md that does not name a crate in it is refused", 1, set_file("crates/adapters/README.md", "memory"),
         "does not name `memory/`")
    case("a group README.md that does not name a group in it is refused", 1,
         set_file("crates/testing/README.md", "`seed/`"), "does not name `fixtures/`")
    case("a group that is also a crate (a manifest beside the README.md) is refused", 1,
         set_file("crates/adapters/Cargo.toml", ""), "crates/adapters/Cargo.toml: a group holds no file")
    case("a normal dependency on a crate in testing/ is refused", 1,
         set_dep("crates/server", "crates/testing/fixtures/seed", None), "normal dependency on thrust-seed")
    case("a build dependency on a crate in spikes/ is refused", 1, set_dep("crates/server", "crates/spikes/probe", "build"),
         "build dependency on thrust-probe")
    case("a normal dependency of core/ on a crate outside core/ is refused", 1,
         set_dep("crates/core/usecases", "crates/adapters/memory", None), "outside core/ (rule 4)")
    case("a build dependency of core/ on a crate outside core/ is refused", 1,
         set_dep("crates/core/types", "crates/adapters/memory", "build"), "build dependency on thrust-memory")
    case("a workflow `paths:` filter naming no file is refused", 1,
         set_file(".github/workflows/deploy.yml", "on:\n  push:\n    paths:\n      - crates/thrust-server/Cargo.toml\n"),
         "crates/thrust-server/Cargo.toml names no tracked file")
    case("a workflow `paths-ignore:` glob over a directory with no file is refused", 1,
         set_file(".github/workflows/deploy.yml", "on:\n  push:\n    paths-ignore:\n      - \".github/actions/gone/**\"\n"),
         "no tracked file is under .github/actions/gone/")
    case("a workflow filter after a comment line is still read", 1,
         set_file(".github/workflows/deploy.yml", "on:\n  push:\n    paths:\n      # a comment\n      - crates/gone.toml\n"),
         "crates/gone.toml names no tracked file")
    case("a mutants.toml exclusion over a directory with no file is refused", 1,
         set_file(".cargo/mutants.toml", 'exclude_globs = ["crates/thrust-testing/**"]\n'), "no tracked file is under crates/thrust-testing/")
    case("a mutants.toml literal exclusion naming no file is refused", 1,
         set_file(".cargo/mutants.toml", 'examine_globs = ["crates/server/src/gone.rs"]\n'), "gone.rs names no tracked file")
    case("a .gitignore line under crates/ inside no crate is refused", 1, set_file(".gitignore", "crates/thrust-server/build/\n"),
         "crates/thrust-server/build/ lies inside no crate")
    case("a .gitignore glob line under crates/ inside no crate is refused", 1, set_file(".gitignore", "crates/gone/.w*.toml\n"),
         "crates/gone/.w*.toml lies inside no crate")
    case("a world with no workspace member is not a pass", 2, lambda f, c: c.clear(), "nothing to check")
    case("a world with no tracked file under crates/ is not a pass", 2,
         lambda f, c: [f.pop(p) for p in [p for p in f if p.startswith("crates/")]], "nothing to check")

    failed = 0
    for label, want, expect, files, crates in cases:
        got, lines = run_world(files, crates)
        hit = [l for l in lines if expect in l] if expect else lines
        if got == want and (want == 0) == (not lines) and (want == 0 or hit):
            print(f"ok: {label} (exit {got}{': ' + hit[0] if hit else ''})")
        else:
            failed = 1
            print(f"FAIL: {label}: exit {got}, wanted {want}; {lines}")
    if not failed:
        print(f"self-test passed: {len(cases)} cases")
    return failed


if __name__ == "__main__":
    if sys.argv[1] == "--self-test":
        sys.exit(self_test())
    sys.exit(git_world(*sys.argv[1:4]))
PY

case "${1:-}" in
  --self-test)
    python3 -I "$TMP/layout.py" --self-test
    exit $?
    ;;
  "")
    git -C "$ROOT" ls-files >"$TMP/files" || { echo "ERROR: git ls-files failed"; exit 2; }
    "$ROOT/scripts/rs" cargo metadata --manifest-path "$ROOT/Cargo.toml" --format-version 1 --no-deps --offline \
      >"$TMP/metadata.json" 2>"$TMP/metadata.err" || {
      echo "ERROR: cargo metadata failed:"
      sed 's/^/    /' "$TMP/metadata.err"
      exit 2
    }
    python3 -I "$TMP/layout.py" "$ROOT" "$TMP/files" "$TMP/metadata.json"
    exit $?
    ;;
  *)
    echo "usage: scripts/check-layout.sh [--self-test]" >&2
    exit 2
    ;;
esac
