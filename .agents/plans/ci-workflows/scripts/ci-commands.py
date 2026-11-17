#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pyyaml>=6.0,<7"]
# ///
"""Do two versions of the workflows run the same steps? The parity check of the CI migration (phases/0002-migration.md).

    uv run --script .agents/plans/ci-workflows/scripts/ci-commands.py --before git:origin/main --after . \
        [--expect-new FILE:JOB,...] [--expect-changed FILE:JOB,...]
    uv run --script .agents/plans/ci-workflows/scripts/ci-commands.py --self-test

A source is a directory (a checkout or worktree) or `git:<ref>`, read from the repository in --root (default: here). It reads every
`.github/workflows/*.yml` and `.github/actions/*/action.yml`, and for each job (a composite action counts as one) takes its
FINGERPRINT: the job's `runs-on`, `timeout-minutes`, `permissions`, `env`, `environment`, `strategy`, `container`, `services`,
`outputs`, job-level `concurrency`, `name`, and every step in order (`name`, `id`, `uses`, `with`, `run`, `shell`, `env`, `if`,
`working-directory`, `continue-on-error`, `timeout-minutes`). It leaves out the job's own `if` and `needs`, which are what a move
changes: `contains(fromJSON(needs.changes.outputs.jobs ...), 'docs')` becomes `contains(fromJSON(inputs.plan), 'docs')`.

A job is the same job wherever it lives, so the two versions are compared by job id:
  moved     the id is in both, in different files, with the same fingerprint        reported, fine
  same      the id is in both, in the same file, with the same fingerprint          fine
  changed   the id is in both with different fingerprints                           a failure unless --expect-changed names it
  appeared  the id is only after                                                    a failure unless --expect-new names it
  gone      the id is only before                                                   always a failure: a refactor drops no job
A job id that exists in two files on one side is matched by (file, id) first and by id alone only when that is unique, so an
ambiguous pair is reported as gone and appeared, never silently paired.

What it does not see: comments (the parser drops them), and what a step does when it runs. It proves that no step was dropped,
added or edited, not that the new workflow works; the probes and the runs do that.

Exit status: 0 parity (every difference was expected); 1 a difference nobody expected; 2 called wrongly or a file that is not YAML.
"""

import argparse
import difflib
import glob
import json
import os
import subprocess
import sys
import tempfile
import textwrap

import yaml

JOB_KEYS = ("runs-on", "timeout-minutes", "permissions", "env", "environment", "strategy", "container", "services", "outputs",
            "concurrency", "name", "uses", "with", "secrets", "continue-on-error", "defaults")
STEP_KEYS = ("name", "id", "uses", "with", "run", "shell", "env", "if", "working-directory", "continue-on-error", "timeout-minutes")


class Usage(Exception):
    pass


def git(root, *args):
    done = subprocess.run(["git", "-C", root, *args], capture_output=True, text=True)
    if done.returncode != 0:
        raise Usage(f"git {' '.join(args)} failed: {done.stderr.strip()}")
    return done.stdout


def read_sources(src, root):
    """{repository path: text} of every workflow and composite action in a source."""
    files = {}
    if src.startswith("git:"):
        ref = src[4:]
        for path in git(root, "ls-tree", "-r", "--name-only", ref, "--", ".github/workflows", ".github/actions").splitlines():
            parts = path.split("/")
            ok_workflow = len(parts) == 3 and parts[1] == "workflows" and path.endswith((".yml", ".yaml"))
            ok_action = len(parts) == 4 and parts[1] == "actions" and parts[3] in ("action.yml", "action.yaml")
            if ok_workflow or ok_action:
                files[path] = git(root, "show", f"{ref}:{path}")
        return files
    if not os.path.isdir(src):
        raise Usage(f"{src} is neither a directory nor git:<ref>")
    patterns = (".github/workflows/*.yml", ".github/workflows/*.yaml", ".github/actions/*/action.yml", ".github/actions/*/action.yaml")
    for pattern in patterns:
        for path in glob.glob(os.path.join(src, pattern)):
            with open(path, encoding="utf-8") as f:
                files[os.path.relpath(path, src)] = f.read()
    return files


def step_print(step):
    return {k: step[k] for k in STEP_KEYS if k in step}


def jobs_of(files):
    """[(file, job id, fingerprint)] for every job and composite action of these files."""
    out = []
    for path in sorted(files):
        try:
            doc = yaml.safe_load(files[path])
        except yaml.YAMLError as e:
            raise Usage(f"{path} is not YAML: {e}")
        if not isinstance(doc, dict):
            raise Usage(f"{path} is not a mapping")
        if "jobs" in doc:
            for jid, job in (doc["jobs"] or {}).items():
                fp = {k: job[k] for k in JOB_KEYS if k in job}
                fp["steps"] = [step_print(s) for s in job.get("steps") or []]
                out.append((os.path.basename(path), jid, fp))
        elif "runs" in doc:
            steps = (doc["runs"] or {}).get("steps") or []
            out.append((os.path.basename(os.path.dirname(path)) + "/action.yml", "action:" + os.path.basename(os.path.dirname(path)),
                        {"steps": [step_print(s) for s in steps]}))
    return out


def pair(before, after):
    """(pairs of (before job, after job), gone, appeared). (file, id) first, then id alone when unique on both sides."""
    after_left = list(after)
    pairs, before_left = [], []
    for b in before:
        match = next((a for a in after_left if a[0] == b[0] and a[1] == b[1]), None)
        if match:
            pairs.append((b, match))
            after_left.remove(match)
        else:
            before_left.append(b)
    gone = []
    for b in before_left:
        same_id_before = [x for x in before_left if x[1] == b[1]]
        candidates = [a for a in after_left if a[1] == b[1]]
        if len(same_id_before) == 1 and len(candidates) == 1:
            pairs.append((b, candidates[0]))
            after_left.remove(candidates[0])
        else:
            gone.append(b)
    return pairs, gone, after_left


def diff(b, a):
    left = json.dumps(b[2], indent=1, sort_keys=True, default=str).splitlines()
    right = json.dumps(a[2], indent=1, sort_keys=True, default=str).splitlines()
    return "\n".join(difflib.unified_diff(left, right, f"{b[0]}:{b[1]} (before)", f"{a[0]}:{a[1]} (after)", lineterm="", n=2))


def compare(before_files, after_files, expect_new=(), expect_changed=()):
    """(exit status, the report lines)."""
    before, after = jobs_of(before_files), jobs_of(after_files)
    pairs, gone, appeared = pair(before, after)
    lines, bad = [], 0
    n_same = n_moved = 0
    for b, a in pairs:
        where = f"{a[0]}:{a[1]}"
        if b[2] == a[2]:
            if b[0] == a[0]:
                n_same += 1
            else:
                n_moved += 1
                lines.append(f"moved    {b[0]}:{b[1]} -> {a[0]}:{a[1]}  ({len(a[2]['steps'])} steps, every one identical)")
        elif where in expect_changed:
            lines.append(f"changed  {where}  (expected)\n" + textwrap.indent(diff(b, a), "           "))
        else:
            bad += 1
            lines.append(f"CHANGED  {where}  (NOT expected)\n" + textwrap.indent(diff(b, a), "           "))
    for b in gone:
        bad += 1
        lines.append(f"GONE     {b[0]}:{b[1]}  ({len(b[2]['steps'])} steps): a refactor drops no job")
    for a in appeared:
        where = f"{a[0]}:{a[1]}"
        if where in expect_new:
            lines.append(f"appeared {where}  (expected, {len(a[2]['steps'])} steps)")
        else:
            bad += 1
            lines.append(f"APPEARED {where}  (NOT expected, {len(a[2]['steps'])} steps)")
    seen = {f"{a[0]}:{a[1]}" for a in appeared}
    for want in expect_new:
        if want not in seen:
            bad += 1
            lines.append(f"MISSING  {want} was expected to appear and did not")
    changed_seen = {f"{a[0]}:{a[1]}" for b, a in pairs if b[2] != a[2]}
    for want in expect_changed:
        if want not in changed_seen:
            bad += 1
            lines.append(f"MISSING  {want} was expected to change and did not")
    lines.append(f"{len(before)} jobs before, {len(after)} after: {n_same} unchanged, {n_moved} moved, {len(changed_seen)} changed, "
                 f"{len(appeared)} appeared, {len(gone)} gone; {'PARITY' if not bad else str(bad) + ' unexpected'}")
    return (1 if bad else 0), lines


def split(value):
    return tuple(x for x in (value or "").split(",") if x)


def self_test():
    """The comparison on hand-made trees, and every refusal: a check that cannot fail is not evidence."""
    ok = bad = 0

    def check(label, cond):
        nonlocal ok, bad
        ok, bad = (ok + 1, bad) if cond else (ok, bad + 1)
        print(("ok   " if cond else "BAD  ") + label)

    def job(text):
        """A job's YAML with no indent, so that jobs can be added and the whole indented once."""
        return textwrap.dedent(text).lstrip("\n")

    def wf(jobs):
        return "name: x\non: push\njobs:\n" + textwrap.indent(jobs, "  ")

    docs = job("""
        docs:
          needs: changes
          if: contains(fromJSON(needs.changes.outputs.jobs), 'docs')
          runs-on: ubuntu-24.04
          steps:
            - uses: actions/checkout@abc
            - run: scripts/check-book.sh
            - run: scripts/check-book.sh --self-test
        """)
    docs_moved = job("""
        docs:
          if: contains(fromJSON(inputs.plan), 'docs')
          runs-on: ubuntu-24.04
          steps:
            - uses: actions/checkout@abc
            - run: scripts/check-book.sh
            - run: scripts/check-book.sh --self-test
        """)
    lint = job("""
        lint:
          runs-on: ubuntu-24.04
          steps:
            - run: cargo fmt --check
        """)
    before = {".github/workflows/ci.yml": wf(docs + lint)}

    def run(after, **kw):
        return compare(before, after, **kw)

    status, _ = run(before)
    check("two identical trees are parity", status == 0)
    status, lines = run({".github/workflows/ci.yml": wf(lint), ".github/workflows/documentation.yml": wf(docs_moved)})
    check("a job moved to another file, its `if` and `needs` changed: parity", status == 0)
    check("and the move is reported", any(l.startswith("moved    ci.yml:docs -> documentation.yml:docs") for l in lines))
    status, lines = run({".github/workflows/ci.yml": wf(lint)})
    check("a job that is gone is refused", status == 1 and any(l.startswith("GONE     ci.yml:docs") for l in lines))
    status, lines = run({".github/workflows/ci.yml": wf(lint), ".github/workflows/documentation.yml": wf(docs_moved.replace(
        "    - run: scripts/check-book.sh --self-test\n", ""))})
    check("a step dropped in the move is refused, and the diff names it",
          status == 1 and any(l.startswith("CHANGED  documentation.yml:docs") and "--self-test" in l for l in lines))
    status, lines = run({".github/workflows/ci.yml": wf(lint), ".github/workflows/documentation.yml": wf(docs_moved.replace(
        "scripts/check-book.sh\n", "scripts/check-book.sh --quiet\n", 1))})
    check("a command changed in the move is refused", status == 1 and any(l.startswith("CHANGED") for l in lines))
    status, _ = run({".github/workflows/ci.yml": wf(lint), ".github/workflows/documentation.yml": wf(docs_moved.replace(
        "actions/checkout@abc", "actions/checkout@def"))})
    check("a pin changed in the move is refused", status == 1)
    status, _ = run({".github/workflows/ci.yml": wf(lint), ".github/workflows/documentation.yml": wf(docs_moved.replace(
        "ubuntu-24.04", "ubuntu-22.04"))})
    check("a runner changed in the move is refused", status == 1)
    extra = job("""
        verdict:
          runs-on: ubuntu-24.04
          steps:
            - run: echo judge
        """)
    moved_tree = {".github/workflows/ci.yml": wf(lint), ".github/workflows/documentation.yml": wf(docs_moved + extra)}
    status, lines = run(moved_tree)
    check("a job that appeared, not named, is refused", status == 1 and any(l.startswith("APPEARED documentation.yml:verdict") for l in lines))
    status, _ = run(moved_tree, expect_new=("documentation.yml:verdict",))
    check("the same job, named in --expect-new, is parity", status == 0)
    status, lines = run(moved_tree, expect_new=("documentation.yml:verdict", "documentation.yml:nothing"))
    check("a job expected to appear that did not is refused", status == 1 and any(l.startswith("MISSING  documentation.yml:nothing") for l in lines))
    gate_before = {".github/workflows/ci.yml": wf(lint)}
    status, lines = compare(gate_before, {".github/workflows/ci.yml": wf(lint.replace("fmt --check", "fmt"))}, expect_changed=("ci.yml:lint",))
    check("a change named in --expect-changed is parity, and shown", status == 0 and any(l.startswith("changed  ci.yml:lint  (expected)") for l in lines))
    status, _ = compare(gate_before, gate_before, expect_changed=("ci.yml:lint",))
    check("a change expected and not made is refused", status == 1)
    # the same id in two files: matched by file and id first
    two_before = {".github/workflows/a.yml": wf(lint), ".github/workflows/b.yml": wf(lint.replace("fmt --check", "clippy"))}
    status, _ = compare(two_before, two_before)
    check("the same job id in two files, both unchanged, is parity", status == 0)
    two_after = {".github/workflows/a.yml": wf(lint.replace("fmt --check", "clippy")), ".github/workflows/b.yml": wf(lint)}
    status, _ = compare(two_before, two_after)
    check("the same two jobs with their contents swapped is refused", status == 1)
    amb_before = {".github/workflows/a.yml": wf(lint), ".github/workflows/b.yml": wf(lint)}
    amb_after = {".github/workflows/c.yml": wf(lint), ".github/workflows/d.yml": wf(lint)}
    status, lines = compare(amb_before, amb_after)
    check("an ambiguous pair is reported as gone and appeared, never paired", status == 1 and sum(l.startswith("GONE") for l in lines) == 2)
    # composite actions
    act = "name: a\nruns:\n  using: composite\n  steps:\n    - run: echo one\n      shell: bash\n"
    status, _ = compare({".github/actions/a/action.yml": act}, {".github/actions/a/action.yml": act})
    check("a composite action unchanged is parity", status == 0)
    status, _ = compare({".github/actions/a/action.yml": act}, {".github/actions/a/action.yml": act.replace("one", "two")})
    check("a composite action's step changed is refused", status == 1)
    # what is not a step
    status, _ = compare(before, {".github/workflows/ci.yml": "# a new comment\n" + wf(docs + lint)})
    check("a comment is not a difference", status == 0)
    # the refusals to read
    try:
        jobs_of({".github/workflows/bad.yml": "jobs: [unclosed"})
        check("a file that is not YAML is refused", False)
    except Usage:
        check("a file that is not YAML is refused", True)
    # reading a directory and a git ref
    with tempfile.TemporaryDirectory() as d:
        os.makedirs(os.path.join(d, ".github", "workflows"))
        with open(os.path.join(d, ".github", "workflows", "ci.yml"), "w", encoding="utf-8") as f:
            f.write(wf(lint))
        os.makedirs(os.path.join(d, ".github", "actions", "a"))
        with open(os.path.join(d, ".github", "actions", "a", "action.yml"), "w", encoding="utf-8") as f:
            f.write(act)
        got = read_sources(d, d)
        check("a directory is read: its workflow and its action", sorted(got) == [".github/actions/a/action.yml", ".github/workflows/ci.yml"])
        subprocess.run(["git", "-C", d, "init", "-q"], check=True)
        subprocess.run(["git", "-C", d, "add", "-A"], check=True)
        subprocess.run(["git", "-C", d, "-c", "user.email=a@b", "-c", "user.name=a", "-c", "commit.gpgsign=false", "commit", "-q", "-m", "x"],
                       check=True)
        check("git:HEAD reads the same two files", sorted(read_sources("git:HEAD", d)) == sorted(got))
        try:
            read_sources("git:nonesuch", d)
            check("a ref that does not exist is refused", False)
        except Usage:
            check("a ref that does not exist is refused", True)
    try:
        read_sources("/nonexistent/dir", ".")
        check("a source that is neither a directory nor a ref is refused", False)
    except Usage:
        check("a source that is neither a directory nor a ref is refused", True)

    print(f"passed={ok} bad={bad}")
    return bad == 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--before", help="a directory, or git:<ref>")
    ap.add_argument("--after", default=".", help="a directory, or git:<ref> (default: this directory)")
    ap.add_argument("--root", default=".", help="the repository that git:<ref> reads (default: this one)")
    ap.add_argument("--expect-new", help="comma-separated FILE:JOB that are expected to appear")
    ap.add_argument("--expect-changed", help="comma-separated FILE:JOB that are expected to change")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        sys.exit(0 if self_test() else 1)
    if not args.before:
        print("ci-commands: --before is needed", file=sys.stderr)
        sys.exit(2)
    try:
        before = read_sources(args.before, args.root)
        after = read_sources(args.after, args.root)
        if not before or not after:
            raise Usage(f"no workflow found in {'--before' if not before else '--after'}")
        status, lines = compare(before, after, split(args.expect_new), split(args.expect_changed))
    except Usage as e:
        print(f"ci-commands: {e}", file=sys.stderr)
        sys.exit(2)
    print("\n".join(lines))
    sys.exit(status)


if __name__ == "__main__":
    main()
