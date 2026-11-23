"""check-json.py over generated documents, generated trees, a control that must be able to fail, and broken copies of itself (the `json` job of
lint.yml; Lint phase, stage 3).

    test_check_json.py [path to a copy of check-json.py]

check_text, over documents generated and not listed one by one:
  * every JSON text in a list of valid forms (scalars, empty containers, odd whitespace, the same key in two different objects, keys that differ
    only in case or in what an escape reads, a number of 5,000 digits, 400 levels of nesting) has no problem;
  * a key twice, at every depth from 0 to 4, in an object that is nested in objects or in arrays, in every spelling of the pair (the same
    characters, an escape and the character, `\\/` and `/`, a surrogate pair and the emoji, `\\n` and `\\u000a`, the empty key), with three kinds
    of value, is "duplicate key", and names the key;
  * a list of texts that are not JSON (a comment, a trailing comma, single quotes, NaN, Infinity, a leading zero, a byte order mark, two
    documents, a raw tab in a string, nothing at all) is a "syntax error", and bytes that are not UTF-8 are "not UTF-8".

The command, over trees generated in a temporary git repository: a clean tree passes and says how many files it read; each kind of defect,
written in every kind of place at once (the top, nested, a name with a blank, a name with dots, a directory that only looks like an excluded
one), fails, names each of those files once and no other, and counts them; what is excluded (web/, the plans, the spikes, probes/, any
node_modules/) is not read though it is broken, and the count says so; a file that is not `*.json` is not read; an untracked file is not read;
a tracked file that is gone is a finding; two keys twice in one file are two findings; a directory that is not a git work tree and a tree
with no JSON to read are exit 2.

The control must be able to fail: check_text is replaced, in the loaded module, by a parser that sees no key twice, one that sees only the
top-level object's, one that reads keys before their escapes are read (it misses `"a"` and `"\\u0061"`), one that takes NaN, one that refuses a
number of 5,000 digits, one that refuses everything and one that accepts everything, over a tree that is clean. Each must make the run exit 1,
saying the control failed.

Then it makes a copy of check-json.py with one bug planted, once for each of the rows of BROKEN, and runs this very test on it: the test must
fail for every one. The test must pass on a copy with no bug first, and the text a bug replaces must be found exactly once, or the answer
could not be trusted (the same rule as mutate_gate.py).

The repository's own files must pass. A path argument points the test at a deliberately broken copy, to show it fails. SHOW_KILLS=1 prints, for
each broken copy, how many lines it failed with and the first of them, to see that it died of the bug and not of something else.
"""

import concurrent.futures
import contextlib
import importlib.util
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
REAL = os.path.abspath(os.path.join(HERE, "..", "check-json.py"))
SCRIPT = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else REAL

passed = 0
failed = 0


def check(label, got, want):
    global passed, failed
    if got == want:
        passed += 1
    else:
        failed += 1
        print(f"BAD  {label}: got {got!r}, wanted {want!r}")


def load():
    spec = importlib.util.spec_from_file_location("check_json_under_test", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


MOD = load()
REAL_CHECK_TEXT = MOD.check_text


def kinds(raw):
    """The kind of each problem check_text finds: which of its three messages it begins with."""
    return [next((k for k in ("duplicate key", "syntax error", "not UTF-8") if m.startswith(k)), m) for m in MOD.check_text(raw)]


# ---- check_text -------------------------------------------------------------------------------------------------------------------

VALID = [
    b"null", b"true", b"0", b"-0.5e+10", b'""', b'"a"', b"[]", b"{}", b"[[],[{}]]", b'{"":1}',
    b'{"a":{"x":1},"b":{"x":2}}', b'{"a":[{"x":1},{"x":2}]}', b'{"a":1,"b":{"a":2}}', b'{"A":1,"a":2}',
    b'{"a":1,"b":1}', b'{"a":null,"b":null}', b'{"a":"\\u0061 and \\u0061"}',
    b'{"\\u00e9":1,"e\\u0301":2}', '{"\u00e9":1,"\u00e8":2}'.encode(), b'{"\\ud800":1}', b'{"n":1E400}',
    b' \r\n\t{"a" :\t1 }\r\n\n\n', b'{"a":   1,\n\n"b":[1,2,\n3]}',
    b'{"n":' + b"1" * 5000 + b"}", b"[" * 400 + b"]" * 400, b'{"a":{"a":{"a":{"a":1}}}}',
]
for raw in VALID:
    check(f"valid: {raw[:50]!r} has no problem", MOD.check_text(raw), [])

SPELLINGS = [
    ('"a"', '"a"'), ('"a"', '"\\u0061"'), ('"\\u0061"', '"a"'), ('"\\/"', '"/"'), ('"\\u00e9"', '"\u00e9"'),
    ('"\\ud83d\\ude00"', '"\U0001f600"'), ('"\\n"', '"\\u000a"'), ('""', '""'),
]
VALUES = ["null", "1", '"v"']


def inside(text, depth, style):
    """text, put inside `depth` levels of objects and arrays (which of the two, at each level, depends on style)."""
    for i in range(depth):
        text = '{"w":' + text + "}" if (i + style) % 2 == 0 else "[1," + text + ",2]"
    return text


for depth in range(5):
    for style in (0, 1):
        for first, second in SPELLINGS:
            for value in VALUES:
                doc = inside('{%s:%s,"o":1,%s:%s}' % (first, value, second, value), depth, style)
                check(f"a key twice ({first} then {second}), depth {depth}, style {style}, value {value}: one duplicate key",
                      kinds(doc.encode()), ["duplicate key"])
check("a key twice names the key", MOD.check_text(b'{"abc":1,"abc":2}'), ['duplicate key "abc" (written on line 1)'])
check("a key twice names every line it is written on", MOD.check_text(b'{\n"a":1,\n"b":2,\n"a":3,\n"a":4\n}'),
      ['duplicate key "a" (written on line 2, 4, 5)'])
check("a key twice, one spelled with an escape, is named with the line of the plain spelling", MOD.check_text(b'{"a":1,"\\u0061":2}'),
      ['duplicate key "a" (written on line 1)'])
check("a key twice, both spelled with the escape, is named with no line", MOD.check_text(b'{"\\u0061":1,"\\u0061":2}'), ['duplicate key "a"'])
check("two keys twice are two problems", len(MOD.check_text(b'{"a":1,"a":2,"b":1,"b":2}')), 2)
check("the same key twice in two objects is named once", MOD.check_text(b'[{"k":1,"k":2},{"k":3,"k":4}]'), ['duplicate key "k" (written on line 1)'])
check("a key three times is one problem", len(MOD.check_text(b'{"a":1,"a":2,"a":3}')), 1)

SYNTAX = [
    b"", b"   \n", b"{", b"}", b'{"a":}', b'{"a":1,}', b"[1,]", b"{'a':1}", b"{a:1}", b'{"a":1}{"b":2}', b'{"a":1} x', b"// c\n{}",
    b'{"a":1 /* c */}', b'{"a":NaN}', b'{"a":Infinity}', b'{"a":-Infinity}', b"[NaN]", b"[01]", b"[1.]", b"[.5]", b"[+1]", b'{"a":"\t"}',
    b'"unterminated', b'{"a":tru}', b"\xef\xbb\xbf{}", b'{"a" 1}', b'{"a":1 "b":2}', b"[1 2]", b'{"a":"\\x41"}', b'{"a":"\\u12"}', b"{1:2}",
    b"nul", b"\x00", b'{"a":1}\x00',
]
for raw in SYNTAX:
    check(f"not JSON: {raw[:40]!r} is one syntax error", kinds(raw), ["syntax error"])
for raw in [b'{"a":"\xff"}', b"\xc3\x28", b"\xef\xbb"]:
    check(f"not UTF-8: {raw!r}", kinds(raw), ["not UTF-8"])
check("a syntax error says where", MOD.check_text(b'{"a":[1,2}\n'), ["syntax error: Expecting ',' delimiter (line 1, column 10)"])

# ---- the command ------------------------------------------------------------------------------------------------------------------


def git(repo, *args):
    subprocess.run(["git", "-C", repo, *args], check=True, capture_output=True)


def make_repo(files, untracked=(), gone=()):
    """A temporary git repository holding `files` ({path: bytes}); `untracked` ones are written and not added; `gone` ones are added, then removed."""
    repo = tempfile.mkdtemp(prefix="check-json-test-")
    git(repo, "init", "-q")
    added = []
    for table, tracked in ((files, True), (dict.fromkeys(untracked, b"{"), False), (dict.fromkeys(gone, b"{}"), True)):
        for name, body in table.items():
            path = os.path.join(repo, name)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "wb") as f:
                f.write(body)
            if tracked:
                added.append(name)
    if added:
        git(repo, "add", "-f", "--", *added)
    for name in gone:
        os.remove(os.path.join(repo, name))
    return repo


def run(root):
    p = subprocess.run([sys.executable, "-I", SCRIPT, root], capture_output=True, text=True)
    return p.returncode, p.stdout, p.stderr


def with_repo(files, **kw):
    repo = make_repo(files, **kw)
    try:
        return run(repo)
    finally:
        shutil.rmtree(repo, ignore_errors=True)


def named(err):
    """The files a run's stderr names, one entry per finding line, in the order given (a line is `name: message`)."""
    return [l.split(": ", 1)[0] for l in err.splitlines() if l and not l.startswith(("check-json:", "  |"))]


CLEAN = {"ok1.json": b'{"a":1}\n', "d/ok2.json": b"[1,2,3]\n", "d/e/ok3.json": b'{"k":{"k":1}}'}

rc, out, err = with_repo(CLEAN)
check("a clean tree exits 0", rc, 0)
check("a clean tree says it read its 3 files", re.match(r"^check-json: 3 files, no finding \(python \d+\.\d+\.\d+\)\n$", out) is not None, True)
check("a clean tree says nothing on stderr", err, "")

DEFECTS = {
    "a key twice": b'{"a":1,"a":2}\n',
    "a key twice, one with an escape": b'{"a":1,"\\u0061":2}\n',
    "a key twice, nested in an array": b'{"l":[{"k":1,"k":2}]}\n',
    "a syntax error": b'{"a":[1,2}\n',
    "NaN": b"[NaN]\n",
    "a byte order mark": b"\xef\xbb\xbf{}\n",
    "bytes that are not UTF-8": b'["\xff"]\n',
    "an empty file": b"",
}
# Every one of these is read: a name that only looks like an excluded one is not excluded.
PLACES = [
    "x.json", "d/x.json", "a/b/c/x.json", ".github/x.json", "dir.with.dots/x.json", "with blank/x y.json", "tools/oracle-v1/x.json",
    "web-app/x.json", "webs/x.json", "tools/spike/x.json", "tools/spikes/x.json", "a/web/x.json", ".agents/skills/x.json",
    ".agents/plans-x/x.json", "node_modules_x/x.json", "xnode_modules/x.json", "probes-x/x.json",
]
for kind, body in DEFECTS.items():
    rc, out, err = with_repo({**CLEAN, **{place: body for place in PLACES}})
    check(f"{kind}, in every place at once: exit 1", rc, 1)
    check(f"{kind}, in every place at once: names each file once, and no other", sorted(named(err)), sorted(PLACES))
    check(f"{kind}, in every place at once: counts {len(PLACES)} findings in {3 + len(PLACES)} files", f"check-json: {3 + len(PLACES)} files, {len(PLACES)} findings;" in err, True)
    check(f"{kind}, in every place at once: says nothing of success", out, "")

EXCLUDED_PLACES = [
    "web/x.json", "web/a/b/x.json", ".agents/plans/p/x.json", ".agents/plans/p/q/r/x.json", "tools/spike-0004/x.json", "tools/spike-0004/w/b/x.json",
    "probes/x.json", "probes/a/b/x.json", "node_modules/x.json", "a/node_modules/x.json", "a/b/node_modules/c/x.json",
]
rc, out, err = with_repo({**CLEAN, **{place: b'{"a":1,"a":2' for place in EXCLUDED_PLACES}})
check("what is excluded, though broken, is not read", (rc, err), (0, ""))
check("what is excluded is not counted", out.startswith("check-json: 3 files, no finding"), True)
NOT_JSON_BY_NAME = ["x.jsonc", "x.json5", "x.jsonl", "x.json.bak", "json", "x.json/y.txt", "d/.json.d/z.yml"]
rc, out, err = with_repo({**CLEAN, **{place: b'{"a":1,"a":2' for place in NOT_JSON_BY_NAME}})
check("a file that is not `*.json` by name, though broken, is not read or counted", (rc, err, out.startswith("check-json: 3 files, no finding")), (0, "", True))

rc, out, err = with_repo(CLEAN, untracked=["u.json", "d/u2.json"])
check("an untracked file is not read", (rc, err, out.startswith("check-json: 3 files, no finding")), (0, "", True))
rc, out, err = with_repo(CLEAN, gone=["gone.json"])
check("a tracked file that is gone is a finding", (rc, named(err), "4 files, 1 finding;" in err), (1, ["gone.json"], True))
rc, out, err = with_repo({**CLEAN, "two.json": b'{"a":1,"a":2,"b":1,"b":2}'})
check("two keys twice in one file are two findings, both named", (rc, err.count("two.json: duplicate key"), "2 findings;" in err), (1, 2, True))

plain = tempfile.mkdtemp(prefix="check-json-plain-")
try:
    rc, out, err = run(plain)
    check("a directory that is not a git work tree is exit 2", (rc, "not a git work tree" in err, out), (2, True, ""))
finally:
    shutil.rmtree(plain, ignore_errors=True)
rc, out, err = with_repo({"web/x.json": b"{}", ".agents/plans/p.json": b"{}", "notes.md": b"x"})
check("a tree whose JSON is all excluded is exit 2", (rc, "no JSON file found" in err, out), (2, True, ""))
rc, out, err = with_repo({"notes.md": b"x"})
check("a tree with no JSON is exit 2", (rc, "no JSON file found" in err, out), (2, True, ""))

# ---- the control must be able to fail ---------------------------------------------------------------------------------------------


def sees_no_key_twice(raw):
    return [m for m in REAL_CHECK_TEXT(raw) if not m.startswith("duplicate key")]


def sees_only_the_top_level(raw):
    problems = sees_no_key_twice(raw)
    if problems:
        return problems
    top = json.loads(raw.decode("utf-8"), object_pairs_hook=lambda items: items, parse_int=str)
    if isinstance(top, list) and top and all(isinstance(p, tuple) for p in top):
        keys = [k for k, _ in top]
        if len(keys) != len(set(keys)):
            return ["duplicate key (top level)"]
    return []


def reads_keys_before_their_escapes(raw):
    return REAL_CHECK_TEXT(re.sub(rb"\\u([0-9a-fA-F]{4})", rb"#\1", raw))


def takes_nan(raw):
    return [] if b"NaN" in raw else REAL_CHECK_TEXT(raw)


def refuses_a_long_number(raw):
    return ["syntax error: too long"] if b"1" * 5000 in raw else REAL_CHECK_TEXT(raw)


def run_main_with(parser, root):
    out, err = io.StringIO(), io.StringIO()
    MOD.check_text = parser
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = MOD.main([root])
    finally:
        MOD.check_text = REAL_CHECK_TEXT
    return rc, out.getvalue(), err.getvalue()


clean_repo = make_repo(CLEAN)
try:
    rc, out, err = run_main_with(REAL_CHECK_TEXT, clean_repo)
    check("the control passes with the real parser, over a clean tree", (rc, err), (0, ""))
    for name, parser in [
        ("sees no key twice", sees_no_key_twice),
        ("sees only the top-level object's keys twice", sees_only_the_top_level),
        ("reads keys before their escapes are read", reads_keys_before_their_escapes),
        ("takes NaN", takes_nan),
        ("refuses a number of 5,000 digits", refuses_a_long_number),
        ("refuses everything", lambda raw: ["syntax error: no"]),
        ("accepts everything", lambda raw: []),
    ]:
        rc, out, err = run_main_with(parser, clean_repo)
        check(f"a parser that {name} fails the control, over a clean tree", (rc, "the control failed" in err, out), (1, True, ""))
finally:
    shutil.rmtree(clean_repo, ignore_errors=True)

# ---- the repository's own files ---------------------------------------------------------------------------------------------------

if subprocess.run(["git", "-C", ROOT, "rev-parse", "--is-inside-work-tree"], capture_output=True).returncode == 0:
    rc, out, err = run(ROOT)
    check("the repository's own JSON passes", (rc, err), (0, ""))
    check("the repository's own JSON is read, and not nothing", re.match(r"^check-json: [1-9]\d* files, no finding", out) is not None, True)

# ---- broken copies of the script --------------------------------------------------------------------------------------------------

# (name, the text, what it becomes). Each text must be in check-json.py exactly once.
BROKEN = [
    ("duplicate keys are not looked for", "object_pairs_hook=pairs, ", ""),
    ("NaN is taken", "parse_constant=refuse_constant, ", ""),
    ("a number of 5,000 digits is refused", ", parse_int=str)", ")"),
    ("a byte order mark is read away", 'raw.decode("utf-8")', 'raw.decode("utf-8-sig")'),
    ("the control is not run", "    failed = control()\n", "    failed = []\n"),
    ("a failed control exits 0", '            print(f"  | {m}", file=sys.stderr)\n        return 1', '            print(f"  | {m}", file=sys.stderr)\n        return 0'),
    ("a finding exits 0", 'they are above", file=sys.stderr)\n        return 1', 'they are above", file=sys.stderr)\n        return 0'),
    ("no JSON to read passes", "    if not files:\n", "    if False:\n"),
    ("a tree that is not a git work tree is not noticed", "    if p.returncode != 0:\n        return None\n", "    if False:\n        return None\n"),
    ("nothing is excluded", 'EXCLUDED = re.compile(r"^(web/|\\.agents/plans/|tools/spike-|probes/)|(^|/)node_modules/")', 'EXCLUDED = re.compile(r"^$^")'),
    ("web/ is excluded wherever it is", 'r"^(web/|', 'r"(web/|'),
    ("the plans are excluded by a prefix of their name", "\\.agents/plans/|", "\\.agents/plans|"),
    ("the spikes are excluded by a prefix of their name", "|tools/spike-|", "|tools/spike|"),
    ("probes/ is excluded by a prefix of its name", "|probes/)|", "|probes)|"),
    ("node_modules/ is excluded by the end of a name", '(^|/)node_modules/")', 'node_modules/")'),
    ("only the top directory is listed", '"--", "*.json"', '"--", ":(glob)*.json"'),
    ("a name with a blank is split", 'p.stdout.split(b"\\0")', 'p.stdout.replace(b"\\0", b" ").split()'),
    ("a file that cannot be read is not a finding", 'file=sys.stderr)\n            findings += 1\n            continue', "file=sys.stderr)\n            continue"),
    ("the first finding stops the run", '            print(f"{name}: {message}", file=sys.stderr)\n            findings += 1\n',
     '            print(f"{name}: {message}", file=sys.stderr)\n            findings += 1\n            return 1\n'),
    ("only the first key twice is reported", "    for key in dict.fromkeys(twice):", "    for key in list(dict.fromkeys(twice))[:1]:"),
]


LAST_WORDS = {}


def suite_on(path):
    p = subprocess.run([sys.executable, "-I", os.path.abspath(__file__), path], capture_output=True, text=True)
    LAST_WORDS[path] = (p.stdout + p.stderr).strip().splitlines()
    return p.returncode


def broken_copy(work, i, source, old, new):
    folder = os.path.join(work, str(i))
    os.mkdir(folder)
    path = os.path.join(folder, "check-json.py")
    with open(path, "w", encoding="utf-8") as f:
        f.write(source.replace(old, new))
    compiled = subprocess.run([sys.executable, "-I", "-m", "py_compile", path], capture_output=True).returncode
    code = suite_on(path)
    return compiled, code, LAST_WORDS[path]


if len(sys.argv) == 1:
    with open(REAL, encoding="utf-8") as f:
        source = f.read()
    work = tempfile.mkdtemp(prefix="check-json-broken-")
    try:
        pristine = os.path.join(work, "check-json.py")
        shutil.copyfile(REAL, pristine)
        check("the suite passes on a copy with no bug planted", suite_on(pristine), 0)
        usable = []
        for i, (name, old, new) in enumerate(BROKEN):
            found = source.count(old)
            check(f"broken copy '{name}': the text it replaces is in check-json.py exactly once", found, 1)
            if found == 1:
                usable.append((i, name, old, new))
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            results = list(pool.map(lambda row: (row[1], broken_copy(work, row[0], source, row[2], row[3])), usable))
        for name, (compiled, code, words) in results:
            check(f"broken copy '{name}' is still Python", compiled, 0)
            check(f"broken copy '{name}' fails the suite", code != 0, True)
            if os.environ.get("SHOW_KILLS"):  # why each one died: its failed checks, and the first of them
                print(f"KILL '{name}': {len(words) - 1} lines; {words[0] if words else ''}")
    finally:
        shutil.rmtree(work, ignore_errors=True)

print(f"test_check_json: {passed} passed, {failed} failed" + (f" (script under test: {SCRIPT})" if SCRIPT != REAL else ""))
sys.exit(1 if failed else 0)
