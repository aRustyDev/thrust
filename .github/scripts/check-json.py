#!/usr/bin/env python3
"""The JSON the repository maintains parses, and no object has a key twice (the `json` job of lint.yml; Lint phase, stage 3).

    python3 -I .github/scripts/check-json.py [<repository root>]     (default: this one)

FILES. What git tracks as `*.json`, less web/ (the stock app's), the records of finished experiments (.agents/plans/, tools/spike-*,
probes/) and any node_modules/. Recorded outputs and generated files are in: a key written twice in a recorded fixture is still a defect,
and package-lock.json is read like the rest. `*.jsonc` is not JSON (it has comments) and none is maintained; the three tracked are the
spikes' wrangler.jsonc.

WHAT FAILS. A file that is not JSON, or has an object with a key twice. "Not JSON" is stricter than Python's json module, which takes
NaN, Infinity and -Infinity: those are refused, as is a byte order mark, a file that is not UTF-8, trailing text, a comment and a trailing
comma. A key twice is a key twice once its escapes are read (`"a"` and `"\\u0061"` are the same key), which is the case a check on the text
misses and biome 2.5.15 measured to miss. Nothing else is a finding: not layout, not key order, not a file without a final newline.

WHY NOT A TOOL. jq and yq take a key written twice (the last wins; measured, exit 0). biome 2.5.15 refuses it, but by default reads 178 of
the 180 files and exits 0 (a file over 1 MiB, corpus.json, is skipped with a warning; a lockfile is never read), and it misses the escaped
duplicate. Python's standard library does the whole of it, in the language every runner image has, with nothing to install or pin.

CONTROL. A check that is not looking passes everything, and a job that reads 180 files with it is green. So before any file is read, the
same function is given inputs it must refuse (a key twice, in an object and in one nested in an array; a key twice in two spellings; a file
that does not parse; NaN) and inputs it must accept (an unstyled file; the same key in two different objects; a number longer than Python
will turn into an int). If any answer is wrong the run fails, whatever the tree holds.

Exit status: 0 the control behaved and every file is clean; 1 a file has a finding, or the control did not behave; 2 called wrongly (not a
git work tree, no JSON to read).
"""

import json
import os
import re
import subprocess
import sys

# Not read: the stock app's, and the records of finished experiments. Recorded outputs and generated files are read.
EXCLUDED = re.compile(r"^(web/|\.agents/plans/|tools/spike-|probes/)|(^|/)node_modules/")


def refuse_constant(name):
    raise ValueError(f"{name} is not JSON")


def lines_of(text, key):
    """The lines on which an object's key is written as `"key":` (best effort: a key written with an escape is not found)."""
    found = re.compile(re.escape(json.dumps(key, ensure_ascii=False)) + r"\s*:")
    return [n for n, line in enumerate(text.splitlines(), 1) if found.search(line)]


def check_text(raw):
    """What is wrong with one file's bytes: a list of messages, empty when it is JSON and no object has a key twice."""
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as e:
        return [f"not UTF-8 ({e.reason}, byte {e.start})"]
    twice = []

    def pairs(items):
        seen = set()
        for key, _ in items:
            if key in seen:
                twice.append(key)
            seen.add(key)
        return dict(items)

    try:
        # parse_int keeps the digits: Python refuses to turn a number of over 4300 digits into an int, and that is valid JSON.
        json.loads(text, object_pairs_hook=pairs, parse_constant=refuse_constant, parse_int=str)
    except json.JSONDecodeError as e:
        return [f"syntax error: {e.msg} (line {e.lineno}, column {e.colno})"]
    except ValueError as e:
        return [f"syntax error: {e}"]
    except RecursionError:
        return ["nested too deep to read"]
    problems = []
    for key in dict.fromkeys(twice):
        lines = lines_of(text, key)
        where = f" (written on line {', '.join(str(n) for n in lines[:5])})" if lines else ""
        problems.append(f"duplicate key {json.dumps(key, ensure_ascii=False)}{where}")
    return problems


# (name, the bytes, what the answer must contain: a word of the message, or None for "no problem").
CONTROL = [
    ("a key twice", b'{"a":1,"b":2,"a":3}\n', "duplicate key"),
    ("a key twice in an object in an array", b'[{"k":1,"k":2}]\n', "duplicate key"),
    ("a key twice, one spelled with an escape", b'{"a":1,"\\u0061":2}\n', "duplicate key"),
    ("a file that does not parse", b'{"a":[1,2}\n', "syntax error"),
    ("NaN, which Python's json module takes", b'{"a":NaN}\n', "syntax error"),
    ("a file that is only unstyled", b'{"a":   1,\n\n"b":[1,2,\n3]}', None),
    ("the same key in two objects", b'{"a":{"x":1},"b":{"x":2},"c":[{"x":3},{"x":4}]}\n', None),
    ("a number of 5,000 digits", b'{"n":' + b"1" * 5000 + b"}\n", None),
]


def control():
    """The messages of the control that failed (empty when check_text answers every case as it must)."""
    failed = []
    for name, raw, want in CONTROL:
        got = check_text(raw)
        if want is None and got:
            failed.append(f"refused {name}, which is JSON: {got}")
        elif want is not None and not any(want in m for m in got):
            failed.append(f"did not refuse {name} (wanted '{want}'): {got}")
    return failed


def tracked(root):
    """The tracked *.json files outside the excluded paths, or None when root is not a git work tree. NUL-separated: a name may hold a blank."""
    p = subprocess.run(["git", "-C", root, "ls-files", "-z", "--", "*.json"], capture_output=True)
    if p.returncode != 0:
        return None
    names = [n.decode("utf-8", "surrogateescape") for n in p.stdout.split(b"\0") if n]
    return [n for n in names if not EXCLUDED.search(n)]


def main(argv):
    root = argv[0] if argv else os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
    files = tracked(root)
    if files is None:
        print(f"check-json: {root} is not a git work tree, so there is no list of the files it tracks", file=sys.stderr)
        return 2
    failed = control()
    if failed:
        print("check-json: the control failed:", file=sys.stderr)
        for m in failed:
            print(f"  | {m}", file=sys.stderr)
        return 1
    if not files:
        print(f"check-json: no JSON file found under {root}: a check of nothing passes, so this fails", file=sys.stderr)
        return 2
    findings = 0
    for name in files:
        try:
            with open(os.path.join(root, name), "rb") as f:
                raw = f.read()
        except OSError as e:
            print(f"{name}: cannot be read: {e.strerror}", file=sys.stderr)
            findings += 1
            continue
        for message in check_text(raw):
            print(f"{name}: {message}", file=sys.stderr)
            findings += 1
    if findings:
        print(f"check-json: {len(files)} files, {findings} finding{'' if findings == 1 else 's'}; they are above", file=sys.stderr)
        return 1
    print(f"check-json: {len(files)} files, no finding (python {sys.version.split()[0]})")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
