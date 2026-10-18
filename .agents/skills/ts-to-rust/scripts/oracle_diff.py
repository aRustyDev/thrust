#!/usr/bin/env -S uv run --offline --script
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""oracle_diff.py - compare recorded reference-implementation responses with the port's replayed ones.

Python standard library only (the PEP 723 block declares no dependencies; with `uv run --offline`
nothing is downloaded, so a local Python 3.10+ is required). Reads the two case directories, the rules
file and the exceptions file named on the command line; writes nothing (it performs NO redaction: it
never modifies or copies the recordings); no network, no subprocess. Deterministic.

  uv run --offline --script scripts/oracle_diff.py compare RECORDED_DIR REPLAY_DIR \
      --rules rules.json [--exceptions exceptions.json] [--max-normalised-pct 50]
  uv run --offline --script scripts/oracle_diff.py selftest

Each case is one JSON file, same name in both directories: {"status": 200, "body": <json>}.
rules.json:      [{"path": "*/id", "as": "<id>", "reason": "server-generated id"}]
exceptions.json: [{"case": "issue_create", "path": "/priority", "reason": "...", "revisit": "..."}]
Paths are JSON-pointer-like; `*` is the ONLY wildcard and matches any run of characters, so `*/id` is an
`id` at any depth while `/id` is only the top-level one. Matching is a plain segment match with no
backtracking: its cost is linear in pattern length x subject length, however many `*` a pattern holds.
`?`, `[` and `]` are rejected in rules and exceptions (they would suggest glob semantics that do not exist).
A rule matches scalar leaf values only; to mask an object, match its fields; a bare-scalar body sits at the
path `/`, in rules and in reports alike. A masked value keeps its JSON type (null, bool, number, string), so a
rule can hide a value but never the difference between null and a string. Every rule and exception needs a reason; every exception needs a concrete `case` and `path`
(wildcard-only values are rejected; the exact path `/` is concrete and names a top-level difference such as a
bare-scalar body or a top-level array length). A difference in the HTTP status is reported at the reserved
path `#status` (a body field named `status` is `/status`). Rules that match no value are reported as WARN
lines.

Exit codes: 0 gate held; 1 gate failed; 2 usage error, malformed case/rule/exception, or unreadable input.
A pass is only meaningful if `selftest` also passes in the same session or CI job.
"""
import argparse
import json
import sys
from pathlib import Path


def _match(pattern, s):
    """`*` is the only wildcard (any run of characters); every other character is literal.

    Leftmost-greedy segment matching, no backtracking: split on `*`; the first part must be a prefix and the
    last a suffix (they may not overlap), and each middle part is located in order inside what remains. At
    most one `str.find` per part, so the cost is bounded by pattern length x subject length.
    """
    parts = pattern.split("*")
    if len(parts) == 1:
        return s == pattern
    first, last = parts[0], parts[-1]
    if len(s) < len(first) + len(last) or not s.startswith(first) or not s.endswith(last):
        return False
    pos, end = len(first), len(s) - len(last)
    for mid in parts[1:-1]:
        i = s.find(mid, pos, end)
        if i < 0:
            return False
        pos = i + len(mid)
    return True


def same_scalar(a, b):
    if isinstance(a, bool) or isinstance(b, bool):
        return type(a) is type(b) and a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return a == b
    return type(a) is type(b) and a == b


def walk(a, b, path, out):
    if isinstance(a, dict) and isinstance(b, dict):
        for k in sorted(set(a) | set(b)):
            p = f"{path}/{k}"
            if k not in b:
                out.append((p, "missing_in_replay"))
            elif k not in a:
                out.append((p, "extra_in_replay"))
            else:
                walk(a[k], b[k], p, out)
    elif isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            out.append((path or "/", "length"))
        for i, (x, y) in enumerate(zip(a, b)):
            walk(x, y, f"{path}/{i}", out)
    elif not same_scalar(a, b):
        out.append((path or "/", "value"))


def _type_name(v):
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "bool"
    if isinstance(v, (int, float)):
        return "number"
    return "string"


def normalise(node, path, rules, counts):
    if isinstance(node, dict):
        return {k: normalise(v, f"{path}/{k}", rules, counts) for k, v in node.items()}
    if isinstance(node, list):
        return [normalise(v, f"{path}/{i}", rules, counts) for i, v in enumerate(node)]
    counts[0] += 1
    where = path or "/"  # a bare-scalar body: the root is "/" here exactly as walk() reports it
    for i, r in enumerate(rules):
        if _match(r["path"], where):
            counts[1] += 1
            counts[2].add(i)
            return f"{r['as']}:{_type_name(node)}"  # masked, but the JSON type is kept
    return node


def _wild_only(s, allow_root=False):
    """True when a pattern is only wildcards and slashes, so it would match (almost) everything.

    The exact path `/` is concrete (it names a top-level difference) and is allowed for exception paths."""
    if allow_root and s == "/":
        return False
    return not str(s).replace("*", "").replace("/", "").strip()


def _bad_glob(s):
    return any(ch in s for ch in "?[]")


def check_inputs(recorded, replay, rules, exceptions):
    """Return an error string for malformed input, else None."""
    for r in rules:
        if not isinstance(r, dict) or not all(isinstance(r.get(k), str) and r.get(k).strip()
                                              for k in ("path", "as", "reason")):
            return f"rule needs non-empty string `path`, `as` and `reason`: {r!r}"
        if _bad_glob(r["path"]):
            return f"rule path may use `*` only; `?`, `[` and `]` are not wildcards here: {r!r}"
    for e in exceptions:
        if not isinstance(e, dict) or not all(isinstance(e.get(k), str) and e.get(k).strip()
                                              for k in ("case", "path", "reason")):
            return f"exception needs non-empty string `case`, `path` and `reason`: {e!r}"
        if _bad_glob(e["path"]) or _bad_glob(e["case"]):
            return f"exception case/path may use `*` only; `?`, `[` and `]` are not wildcards here: {e!r}"
        if _wild_only(e["path"], allow_root=True) or _wild_only(e["case"]):
            return f"exception with a wildcard-only case or path would hide every difference: {e!r}"
    for label, cases in (("recorded", recorded), ("replay", replay)):
        for name, c in cases.items():
            if (not isinstance(c, dict) or "body" not in c or isinstance(c.get("status"), bool)
                    or not isinstance(c.get("status"), int)):
                return f"{label} case {name!r} must be an object with an integer `status` and a `body`"
    return None


def run(recorded, replay, rules, exceptions, max_norm_pct=50.0):
    """recorded/replay: {case_name: {"status":..,"body":..}}. Returns (exit_code, lines)."""
    problem = check_inputs(recorded, replay, rules, exceptions)
    if not problem and not (0.0 <= max_norm_pct <= 100.0):  # also rejects nan
        problem = f"--max-normalised-pct must be between 0 and 100, got {max_norm_pct}"
    if problem:
        return 2, [f"error: {problem}"]
    lines, failed = [], False
    if not recorded:
        return 1, ["RESULT FAIL zero_cases (nothing was compared; a gate that compared nothing proves nothing)"]
    only_rec, only_rep = sorted(set(recorded) - set(replay)), sorted(set(replay) - set(recorded))
    for n in only_rec:
        lines.append(f"FAIL {n}: recorded but not replayed")
    for n in only_rep:
        lines.append(f"FAIL {n}: replayed but never recorded")
    failed = bool(only_rec or only_rep)
    used, used_rules, total, normed, excepted = set(), set(), 0, 0, 0
    for name in sorted(set(recorded) & set(replay)):
        rec, rep = recorded[name], replay[name]
        diffs = []
        if rec.get("status") != rep.get("status"):
            diffs.append(("#status", "status"))  # reserved path: a body field named `status` is /status
        counts = [0, 0, used_rules]
        a = normalise(rec.get("body"), "", rules, counts)
        b = normalise(rep.get("body"), "", rules, counts)
        total, normed = total + counts[0], normed + counts[1]
        walk(a, b, "", diffs)
        live = []
        for p, kind in diffs:
            hit = [i for i, e in enumerate(exceptions)
                   if _match(e["case"], name) and _match(e["path"], p)]
            if hit:
                used.update(hit)
                excepted += 1
            else:
                live.append((p, kind))
        lines.append(f"{'FAIL' if live else 'PASS'} {name}")
        for p, kind in live[:10]:
            lines.append(f"  {kind} at {p}")
        failed = failed or bool(live)
    stale = [exceptions[i] for i in range(len(exceptions)) if i not in used]
    for e in stale:
        lines.append(f"STALE exception matched nothing: case={e['case']} path={e['path']}")
    for i, r in enumerate(rules):
        if i not in used_rules:
            lines.append(f"WARN unused rule matched no value: path={r['path']}")
    pct = 100.0 * normed / total if total else 0.0
    if pct > max_norm_pct:
        lines.append(f"over-normalised: {pct:.1f}% of compared values were replaced by rules (limit {max_norm_pct}%)")
    failed = failed or bool(stale) or pct > max_norm_pct
    lines.append(f"RESULT {'FAIL' if failed else 'PASS'} cases={len(recorded)} values={total} "
                 f"normalised={normed} excepted={excepted} stale_exceptions={len(stale)}")
    return (1 if failed else 0), lines


RULES = [{"path": "/id", "as": "<id>", "reason": "server-generated id"},
         {"path": "/*_at", "as": "<ts>", "reason": "server clock"}]


def selftest():
    base = {"c": {"status": 200, "body": {"id": "a1", "name": "x", "n": 1, "tags": ["p", "q"],
                                          "created_at": "2026-01-01T00:00:00Z", "meta": None}}}

    def variant(**over):
        body = json.loads(json.dumps(base["c"]["body"]))
        body.update(over)
        return {"c": {"status": 200, "body": body}}

    def gone(key):
        v = variant()
        del v["c"]["body"][key]
        return v

    wrong_status = variant()
    wrong_status["c"]["status"] = 500
    cases = [
        ("identical -> PASS", run(base, base, RULES, []), 0),
        ("volatile fields differ only -> PASS", run(base, variant(id="zz", created_at="2027"), RULES, []), 0),
        ("int 1 vs float 1.0 -> PASS", run(base, variant(n=1.0), RULES, []), 0),
        ("MUST-FAIL CONTROL: one value changed -> FAIL", run(base, variant(name="y"), RULES, []), 1),
        ("field missing in replay -> FAIL", run(base, gone("name"), RULES, []), 1),
        ("extra field in replay -> FAIL", run(base, variant(extra=1), RULES, []), 1),
        ("null vs missing are different -> FAIL", run(base, gone("meta"), RULES, []), 1),
        ("array element changed -> FAIL", run(base, variant(tags=["p", "z"]), RULES, []), 1),
        ("array length changed -> FAIL", run(base, variant(tags=["p"]), RULES, []), 1),
        ("status changed -> FAIL", run(base, wrong_status, RULES, []), 1),
        ("true vs 1 are different -> FAIL", run(variant(n=True), variant(n=1), RULES, []), 1),
        ("difference covered by a reasoned exception -> PASS",
         run(base, variant(name="y"), RULES, [{"case": "c", "path": "/name", "reason": "known gap"}]), 0),
        ("exception for /name does not hide a difference at /n -> FAIL",
         run(base, variant(name="y", n=2), RULES, [{"case": "c", "path": "/name", "reason": "known gap"}]), 1),
        ("exception for another case does not apply -> FAIL",
         run(base, variant(name="y"), RULES, [{"case": "other", "path": "/name", "reason": "known gap"}]), 1),
        ("exception that matches nothing is stale -> FAIL",
         run(base, base, RULES, [{"case": "c", "path": "/name", "reason": "old"}]), 1),
        ("zero cases -> FAIL", run({}, {}, RULES, []), 1),
        ("recorded case never replayed -> FAIL", run(base, {}, RULES, []), 1),
        ("rule that erases everything -> FAIL (over-normalisation)",
         run(base, variant(name="y"), [{"path": "/*", "as": "<x>", "reason": "bad rule"}], []), 1),
        ("rule without a reason -> usage error", run(base, base, [{"path": "/id", "as": "<id>"}], []), 2),
        ("masked field: null vs string -> FAIL (the mask keeps the type)", run(base, variant(id=None), RULES, []), 1),
        ("masked field: number vs string -> FAIL", run(base, variant(id=5), RULES, []), 1),
        ("replayed case that was never recorded -> FAIL", run(base, {**base, "extra_case": base["c"]}, RULES, []), 1),
        ("exception without a case -> usage error",
         run(base, variant(name="y"), RULES, [{"path": "/name", "reason": "known gap"}]), 2),
        ("exception with a wildcard-only path -> usage error",
         run(base, variant(name="y"), RULES, [{"case": "c", "path": "*", "reason": "known gap"}]), 2),
        ("exception with a wildcard-only case -> usage error",
         run(base, variant(name="y"), RULES, [{"case": "*", "path": "/name", "reason": "known gap"}]), 2),
        ("case file without status and body -> usage error", run({"c": {}}, {"c": {}}, RULES, []), 2),
        ("case file with a status but no body -> usage error",
         run({"c": {"status": 200}}, {"c": {"status": 200}}, RULES, []), 2),
        ("non-integer status -> usage error",
         run({"c": {"status": "200", "body": {}}}, {"c": {"status": "200", "body": {}}}, RULES, []), 2),
        ("exception path with `?` -> usage error (`*` is the only wildcard)",
         run(base, variant(name="y"), RULES, [{"case": "c", "path": "/?*", "reason": "known gap"}]), 2),
        ("exception path with a bracket class -> usage error",
         run(base, variant(name="y"), RULES, [{"case": "c", "path": "/[!x]*", "reason": "known gap"}]), 2),
        ("exception case with `?` -> usage error",
         run(base, variant(name="y"), RULES, [{"case": "?", "path": "/name", "reason": "known gap"}]), 2),
        ("rule path with `?` -> usage error", run(base, base, [{"path": "/i?", "as": "<id>", "reason": "r"}], []), 2),
        ("an exception for the body field /status does not hide an HTTP status difference -> FAIL",
         run(base, wrong_status, RULES, [{"case": "c", "path": "/status", "reason": "known gap"}]), 1),
        ("HTTP status difference covered by an exception at #status -> PASS",
         run(base, wrong_status, RULES, [{"case": "c", "path": "#status", "reason": "known gap"}]), 0),
        ("a body field named status that differs is not hidden by #status -> FAIL",
         run({"c": {"status": 200, "body": {"status": "ok"}}}, {"c": {"status": 200, "body": {"status": "failed"}}},
             RULES, [{"case": "c", "path": "#status", "reason": "known gap"}]), 1),
        ("--max-normalised-pct nan -> usage error", run(base, base, RULES, [], float("nan")), 2),
        ("--max-normalised-pct above 100 -> usage error", run(base, base, RULES, [], 101.0), 2),
        ("--max-normalised-pct below 0 -> usage error", run(base, base, RULES, [], -1.0), 2),
        ("a rule that matches no value is reported but does not fail",
         run(base, base, RULES + [{"path": "*/nothing", "as": "<n>", "reason": "r"}], []), 0,
         lambda lines: any("WARN unused rule" in line and "*/nothing" in line for line in lines)),
        ("a rule path is literal apart from `*`: `.` does not match `X` -> FAIL",
         run({"c": {"status": 200, "body": {"aXb": 1, "k1": 0, "k2": 0}}},
             {"c": {"status": 200, "body": {"aXb": 2, "k1": 0, "k2": 0}}},
             [{"path": "/a.b", "as": "<v>", "reason": "r"}], []), 1),
        ("a rule path matches a literal `.` -> PASS",
         run({"c": {"status": 200, "body": {"a.b": 1, "k1": 0, "k2": 0}}},
             {"c": {"status": 200, "body": {"a.b": 2, "k1": 0, "k2": 0}}},
             [{"path": "/a.b", "as": "<v>", "reason": "r"}], []), 0),
        ("matcher: prefix, suffix and middles in order -> True",
         (0 if _match("a*b*c", "aXbYc") and _match("/*/id", "/x/id") and _match("*/id", "/r/0/id") else 1, []), 0),
        ("matcher: prefix and suffix may not overlap; middles must follow in order -> False",
         (0 if not (_match("ab*ba", "aba") or _match("a*b*c", "acb") or _match("/*/id", "/id")) else 1, []), 0),
        ("matcher: the prefix is checked (a*c vs bac) -> False",
         (0 if not _match("a*c", "bac") and _match("a*c", "abc") else 1, []), 0),
        ("matcher: a middle is not found inside the prefix (a*a*c vs abc) -> False",
         (0 if not _match("a*a*c", "abc") and _match("a*a*c", "aac") else 1, []), 0),
        ("matcher: a middle is not found inside the suffix (a*c*c vs abc) -> False",
         (0 if not _match("a*c*c", "abc") and _match("a*c*c", "acc") else 1, []), 0),
        ("matcher: the search resumes after a middle (a*b*b*c vs abc) -> False",
         (0 if not _match("a*b*b*c", "abc") and _match("a*b*b*c", "abbc") else 1, []), 0),
        ("matcher: a pattern without `*` is equality -> True and False",
         (0 if _match("/id", "/id") and not _match("/id", "/id2") and not _match("/id", "/x/id") else 1, []), 0),
        ("matcher: 41 stars against a 200-character subject returns (no backtracking)",
         (0 if not _match("*a" * 40 + "*b", "a" * 200) and _match("*a" * 40 + "*b", "a" * 199 + "b") else 1, []), 0),
        ("matcher: 40 stars + a missing middle (`*a`x40 + `*c*b`) over 199 a's and a b returns promptly -> False; "
         "with the middle present -> True",
         (0 if not _match("*a" * 40 + "*c*b", "a" * 199 + "b")
          and _match("*a" * 40 + "*c*b", "a" * 100 + "c" + "a" * 50 + "b") else 1, []), 0),
        ("a rule at `/` masks a bare-scalar body (abc vs xyz; the one value is masked, so the cap is lifted) -> PASS, "
         "and the rule counts as used",
         run({"c": {"status": 200, "body": "abc"}}, {"c": {"status": 200, "body": "xyz"}},
             [{"path": "/", "as": "<v>", "reason": "r"}], [], 100.0), 0,
         lambda lines: not any("WARN unused rule" in line for line in lines)),
        ("the same bare-scalar difference without a rule -> FAIL reported at `/`",
         run({"c": {"status": 200, "body": "abc"}}, {"c": {"status": 200, "body": "xyz"}}, [], []), 1,
         lambda lines: any("value at /" == line.strip() for line in lines)),
        ("a rule at `/` keeps the type of a bare-scalar body (null vs string) -> FAIL on the value, not the cap",
         run({"c": {"status": 200, "body": None}}, {"c": {"status": 200, "body": "xyz"}},
             [{"path": "/", "as": "<v>", "reason": "r"}], [], 100.0), 1,
         lambda lines: any("value at /" == line.strip() for line in lines)),
        ("a rule path with 41 stars over a 200-character key is handled; the rule is unused -> PASS with WARN",
         run({"c": {"status": 200, "body": {"a" * 200: 1}}}, {"c": {"status": 200, "body": {"a" * 200: 1}}},
             [{"path": "*a" * 40 + "*b", "as": "<v>", "reason": "r"}], []), 0,
         lambda lines: any("WARN unused rule" in line for line in lines)),
        ("exception at the concrete path `/` registers a top-level difference -> PASS",
         run({"c": {"status": 200, "body": 5}}, {"c": {"status": 200, "body": 6}}, RULES,
             [{"case": "c", "path": "/", "reason": "known gap"}]), 0),
        ("exception at `/` does not hide a difference at /name -> FAIL",
         run(base, variant(name="y"), RULES, [{"case": "c", "path": "/", "reason": "known gap"}]), 1),
        ("exception at `//` is still wildcard-like -> usage error",
         run(base, variant(name="y"), RULES, [{"case": "c", "path": "//", "reason": "known gap"}]), 2),
        ("exception case `/` is not a concrete case name -> usage error",
         run(base, variant(name="y"), RULES, [{"case": "/", "path": "/name", "reason": "known gap"}]), 2),
    ]
    ok = True
    for label, (code, lines), want, *extra in cases:
        good = code == want and all(check(lines) for check in extra)
        ok = ok and good
        print(("ok     " if good else "NOT OK ") + f"{label} (exit {code}, wanted {want})")
    print("SELFTEST " + ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


def load_dir(d):
    p = Path(d)
    if not p.is_dir():
        raise OSError(f"{d}: not a directory")
    out = {}
    for f in sorted(p.glob("*.json")):
        try:
            out[f.stem] = json.loads(f.read_text())
        except ValueError as e:
            raise ValueError(f"{f.name}: invalid JSON ({e})")
    return out


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("compare")
    c.add_argument("recorded")
    c.add_argument("replay")
    c.add_argument("--rules", required=True)
    c.add_argument("--exceptions")
    c.add_argument("--max-normalised-pct", type=float, default=50.0)
    sub.add_parser("selftest")
    args = ap.parse_args(argv)
    if args.cmd == "selftest":
        return selftest()
    try:
        rules = json.loads(Path(args.rules).read_text())
        exceptions = json.loads(Path(args.exceptions).read_text()) if args.exceptions else []
        if not isinstance(rules, list) or not isinstance(exceptions, list):
            raise ValueError("rules and exceptions files must each contain a JSON array")
        code, lines = run(load_dir(args.recorded), load_dir(args.replay), rules, exceptions,
                          args.max_normalised_pct)
    except (OSError, ValueError, KeyError, TypeError, AttributeError, RecursionError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    print("\n".join(lines))
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
