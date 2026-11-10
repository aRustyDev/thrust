#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pyyaml"]
# ///
"""Compare a recording of Thrust with the golden corpus recorded from the pinned Plane (thrust issue #58).

`record_corpus.py` drives the same scripted scenarios against any target and writes the NORMALISED exchanges
(ids symbolic, times and dates erased, headers whitelisted). Pointed at Thrust (`--out-dir <dir>`), it gives a recording that
should equal the committed corpus, exchange by exchange, for the operations Thrust serves. This script says whether it does.

  compare_corpus.py --recorded <dir of Thrust's recording> --scope m1 [--committed corpus/] [--register exceptions.yaml]

What is compared. Only the exchanges that fall on the operations of the scope (the register's `scopes.<name>.tags`, matched
against the pinned schema by method and path template): the rest of the scenarios (comments, links, activities, relations, the
auth probes on `/users/me/`) exercise operations Thrust does not have. For each: the response status, its kept headers and its
body, and the request path (the symbolic ids and the `<uuid:N>` ordinals in it say that a step created what it did on the pin).
An array is compared in order (normalise.yaml: `ordering: strict`); an object by key, so a key's order is not behaviour.

What is accepted. A difference is accepted only by an entry of the register with `applies_to: corpus` that names its resource,
and its exchange, and (optionally) a regex over the JSON path of the difference. Nothing else is accepted, and an entry that
accepts nothing is STALE and fails the run: an exception nobody needs any more must be removed on purpose.

What a pass needs. A run that compares fewer exchanges than the scope's floor (`corpus.min_exchanges_compared`), or none, fails as
VACUOUS, and so does an exchange the scope expects that the recording lacks.

Exit status: 0 PASS, 1 FAIL, 2 the harness could not run.
"""

import argparse
import collections
import json
import re
import sys
from pathlib import Path

import yaml

HERE = Path(__file__).resolve().parent
ORACLE = HERE.parent
sys.path.insert(0, str(HERE))
from run_conformance import load_register, load_scope, scope_operations  # noqa: E402


def op_regexes(ops):
    """`METHOD /path/{x}/` operation labels as (method, compiled regex over a concrete path) pairs.

    The corpus paths are normalised (`<project:alpha>`, `<issue:i3>`, `<uuid:3>`), so each `{name}` is one path segment, and the
    `{project_identifier}-{issue_identifier}` form of the lookup is two segments' worth of text joined by a hyphen."""
    out = []
    for label in ops:
        method, _, path = label.partition(" ")
        pattern = re.sub(r"\\\{[^}]*\\\}", "[^/]+", re.escape(path))
        out.append((label, method, re.compile("^" + pattern + "$")))
    return out


def operation_of(method, path, regexes):
    """The in-scope operation an exchange falls on, or None. Of several, the one with the fewest placeholders (the most specific)."""
    hits = [(label, rx) for label, m, rx in regexes if m == method and rx.match(path)]
    if not hits:
        return None
    return min(hits, key=lambda h: h[1].pattern.count("[^/]+"))[0]


def canonical(v):
    return json.dumps(v, sort_keys=True)


class Ordering:
    """The register entries of `kind: ordering` that apply to one exchange: arrays (by the JSON path with its indexes erased, such as
    `$.response.body.results[].assignees`) that may come in another order than the pin's. An entry allows a REORDERING and nothing
    else: two arrays that hold the same elements are no difference, and any other difference in them is compared as usual."""

    def __init__(self, entries, resource, exchange):
        self.entries = [e for e in entries if e["kind"] == "ordering" and accepts(e, resource, exchange, None)]
        self.used = set()
        self._reordered = set()

    def entry_for(self, path):
        shape = re.sub(r"\[\d+\]", "[]", path)
        for e in self.entries:
            if re.search(e["match"]["array_path_regex"], shape):
                return e["id"]
        return None

    def canon(self, value, shape):
        """`value` with every array that an entry lets come in another order put in one order, at every depth (so that rows that differ
        only in the order of their own people are the same row)."""
        if isinstance(value, dict):
            return {k: self.canon(v, f"{shape}.{k}") for k, v in value.items()}
        if isinstance(value, list):
            items = [self.canon(v, f"{shape}[]") for v in value]
            entry = self.entry_for(shape)
            if entry:
                ordered = sorted(items, key=canonical)
                if ordered != items:
                    self._reordered.add(entry)
                items = ordered
            return items
        return value

    def same_elements(self, path, committed, recorded):
        """Whether the arrays at `path` hold the same elements, in another order, and an entry allows that. A reordering that was
        needed counts the entry (and the entries of the arrays inside) as used."""
        entry = self.entry_for(path)
        if not entry:
            return False
        shape = re.sub(r"\[\d+\]", "[]", path)
        was = sorted(canonical(self.canon(v, f"{shape}[]")) for v in committed)
        now = sorted(canonical(self.canon(v, f"{shape}[]")) for v in recorded)
        if was != now:
            return False
        if committed != recorded:
            self.used.add(entry)
            self.used |= self._reordered
        return True


def diffs(committed, recorded, path="$", order=None):
    """The differences between two JSON values as (json path, committed, recorded). Objects by key, arrays in order, except the arrays
    that `order` (an `Ordering`) allows to be reordered; it records which entry such a reordering used."""
    if isinstance(committed, dict) and isinstance(recorded, dict):
        for k in sorted(set(committed) | set(recorded)):
            if k not in recorded:
                yield (f"{path}.{k}", committed[k], "<absent>")
            elif k not in committed:
                yield (f"{path}.{k}", "<absent>", recorded[k])
            else:
                yield from diffs(committed[k], recorded[k], f"{path}.{k}", order)
    elif isinstance(committed, list) and isinstance(recorded, list):
        if order and order.same_elements(path, committed, recorded):
            return
        if len(committed) != len(recorded):
            yield (f"{path}.length", len(committed), len(recorded))
        for i, (a, b) in enumerate(zip(committed, recorded)):
            yield from diffs(a, b, f"{path}[{i}]", order)
    elif committed != recorded or type(committed) is not type(recorded):
        yield (path, committed, recorded)


def exchange_view(e):
    """What of an exchange is compared: the request path and the answer."""
    return {"path": e["request"]["path"], "response": e["response"]}


UUID_ORDINAL = re.compile(r"<uuid:(\d+)>")


def renumber(views):
    """The views with their `<uuid:N>` ordinals numbered again by first appearance in THESE views, in order.

    The normaliser numbers an id the run made by first appearance in the whole recording of a resource, so a step that the target does
    not have (a comment, a link: Thrust has no such operation) shifts every ordinal after it. Numbering the compared exchanges alone,
    the same way on both sides, takes that away and keeps what matters: which ids an exchange names, and that the same id is the same."""
    mapping = {}
    out = []
    for view in views:
        def number(mo):
            return f"<uuid:{mapping.setdefault(mo.group(1), len(mapping) + 1)}>"
        out.append(json.loads(UUID_ORDINAL.sub(number, json.dumps(view, sort_keys=True))))
    return out


def corpus_entries(reg, target):
    out = []
    for e in reg.get("entries") or []:
        if e.get("applies_to") != "corpus" or e.get("status", "active") != "active":
            continue
        if target not in (e.get("targets") or [target]):
            continue
        for k in ("id", "kind", "match", "reason", "rust_expectation"):
            if k not in e:
                raise SystemExit(f"entry {e.get('id')!r} is missing required key {k!r}")
        if "resource" not in e["match"]:
            raise SystemExit(f"corpus entry {e['id']} match needs a `resource`")
        out.append(e)
    return out


def accepts(entry, resource, exchange, jpath):
    """Whether `entry` names this resource and exchange and, for a `path_regex`, the JSON path `jpath` of a difference (an entry of
    `kind: ordering` has an `array_path_regex` instead, which `Ordering` uses, so it accepts no difference by itself)."""
    m = entry["match"]
    resources = [m["resource"]] if isinstance(m["resource"], str) else m["resource"]
    if resource not in resources:
        return False
    if m.get("exchanges") and exchange not in m["exchanges"]:
        return False
    if jpath is None:
        return True
    if entry["kind"] == "ordering":
        return False
    if m.get("path_regex") and not re.search(m["path_regex"], jpath):
        return False
    return True


def compare(committed_docs, recorded_docs, resources, regexes, entries, floor):
    """The comparison itself: the committed exchanges of each of `resources` against the recorded ones (both as `{resource: doc}`),
    with the register `entries`, and the `floor` of exchanges that must be compared. Returns the result as a dict."""
    compared, expected_missing = 0, []
    unaccepted, accepted = [], []
    hits = collections.Counter()  # (entry id, exchange) -> differences it accepted
    by_resource = {}
    out_of_scope = collections.Counter()
    for resource in resources:
        committed = committed_docs[resource]["exchanges"]
        recorded = {e["name"]: e for e in recorded_docs[resource]["exchanges"]}
        pairs = []
        for c in committed:
            req = c["request"]
            op = operation_of(req["method"], req["path"], regexes)
            if op is None:
                out_of_scope[resource] += 1
                continue
            r = recorded.get(c["name"])
            if r is None:
                expected_missing.append(f"{resource}/{c['name']}")
                continue
            pairs.append((c, r, op))
        # the ordinals of the ids that the run made are numbered again over the exchanges that are compared (see `renumber`)
        was_views = renumber([exchange_view(c) for c, _, _ in pairs])
        now_views = renumber([exchange_view(r) for _, r, _ in pairs])
        for (c, _, op), was_view, now_view in zip(pairs, was_views, now_views):
            order = Ordering(entries, resource, c["name"])
            found = list(diffs(was_view, now_view, order=order))
            for eid in order.used:
                hits[(eid, c["name"])] += 1
            for jpath, was, now in found:
                d = {"resource": resource, "exchange": c["name"], "operation": op, "path": jpath, "committed": was, "recorded": now}
                hit = [e for e in entries if accepts(e, resource, c["name"], jpath)]
                if hit:
                    accepted.append({**d, "entry": hit[0]["id"]})
                    for e in hit:
                        hits[(e["id"], c["name"])] += 1
                else:
                    unaccepted.append(d)
        by_resource[resource] = len(pairs)
        compared += len(pairs)

    stale = [f"{e['id']}" for e in entries if not e.get("optional") and not any(k[0] == e["id"] for k in hits)]
    vacuous = []
    if compared < floor:
        vacuous.append(f"exchanges compared {compared} < {floor}")
    vacuous += [f"the recording lacks {m}" for m in expected_missing]
    return {
        "exchanges_compared": compared, "by_resource": by_resource, "out_of_scope_exchanges": dict(out_of_scope),
        "accepted": len(accepted), "unaccepted": unaccepted, "stale_entries": stale, "vacuous": vacuous,
        "accepted_by_entry_and_exchange": {f"{k[0]} {k[1]}": v for k, v in sorted(hits.items())},
    }


def verdict_ok(result):
    return not result["unaccepted"] and not result["stale_entries"] and not result["vacuous"]


def print_result(label, result, show):
    print(f"[{label}] exchanges compared={result['exchanges_compared']} {dict(result['by_resource'])} "
          f"out-of-scope={sum(result['out_of_scope_exchanges'].values())} accepted={result['accepted']} "
          f"unaccepted={len(result['unaccepted'])} stale={len(result['stale_entries'])} vacuous={len(result['vacuous'])}")
    shown = collections.OrderedDict()
    for d in result["unaccepted"]:
        shown.setdefault((d["resource"], d["exchange"]), []).append(d)
    for (resource, exchange), ds in list(shown.items())[:show]:
        print(f"  DIFFERS {resource}/{exchange} ({ds[0]['operation']}): {len(ds)} difference(s)")
        for d in ds[:4]:
            print(f"      {d['path']}: committed={json.dumps(d['committed'])[:90]} recorded={json.dumps(d['recorded'])[:90]}")
    for s in result["stale_entries"]:
        print(f"  STALE corpus entry (accepted nothing): {s}")
    for v in result["vacuous"]:
        print(f"  VACUOUS: {v}")


# --- the must-fail controls ---------------------------------------------------------------------------------------------------------

def find(doc, name):
    return next(e for e in doc["exchanges"] if e["name"] == name)


def mutations():
    """Each control: a name, what it does to a COPY of the committed corpus (the recording it stands for), and where the comparison must
    say the difference is. Every one is a defect that a recording of a target could have and that the comparison has to catch; the
    first is the one the issue names (a state create that answers 201 where the pin answers 200)."""
    def state_create_201(docs):
        find(docs["states"], "create")["response"]["status"] = 201

    def drop_an_id(docs):  # the schema cannot see this (`id` is not required there): only the corpus can
        del find(docs["states"], "list")["response"]["body"]["results"][0]["id"]

    def reorder_a_list(docs):
        results = find(docs["projects"], "list")["response"]["body"]["results"]
        results[0], results[1] = results[1], results[0]

    def an_extra_key(docs):
        find(docs["labels"], "detail")["response"]["body"]["added_by_a_defect"] = True

    def a_changed_value(docs):
        find(docs["projects"], "detail")["response"]["body"]["name"] = "Another Name"

    def lose_the_allow_header(docs):
        del find(docs["labels"], "list")["response"]["headers"]["allow"]

    def a_status_that_is_a_string(docs):  # a type difference with equal text
        find(docs["labels"], "list")["response"]["status"] = "200"

    def an_exchange_is_missing(docs):
        docs["projects"]["exchanges"] = [e for e in docs["projects"]["exchanges"] if e["name"] != "detail"]

    return [
        ("a state create that answers 201", state_create_201, "states/create", "$.response.status"),
        ("a response that has lost its id", drop_an_id, "states/list", ".id"),
        ("a list in another order", reorder_a_list, "projects/list", ".results["),
        ("a response with a key more", an_extra_key, "labels/detail", "added_by_a_defect"),
        ("a value that is not the pin's", a_changed_value, "projects/detail", ".name"),
        ("an answer without its Allow header", lose_the_allow_header, "labels/list", "headers.allow"),
        ("a status of the wrong type", a_status_that_is_a_string, "labels/list", "$.response.status"),
        ("an exchange the recording lacks", an_exchange_is_missing, "projects/detail", "the recording lacks"),
    ]


def self_test(committed_docs, resources, regexes, floor):
    """The comparison against itself passes with no register at all, and against each of [`mutations`] it FAILS, naming the exchange.
    A comparison that cannot fail says nothing, so this runs in the gate before a recording is trusted."""
    import copy

    ok = True
    identity = compare(committed_docs, copy.deepcopy(committed_docs), resources, regexes, [], floor)
    passed = verdict_ok(identity) and identity["exchanges_compared"] >= floor
    print(f"  control 0  the corpus against itself: {'PASS (as it must)' if passed else 'FAILED, which it must not'} "
          f"({identity['exchanges_compared']} exchanges, {len(identity['unaccepted'])} differences)")
    ok &= passed
    for n, (name, mutate, where, needle) in enumerate(mutations(), start=1):
        docs = copy.deepcopy(committed_docs)
        mutate(docs)
        result = compare(committed_docs, docs, resources, regexes, [], floor)
        text = json.dumps(result["unaccepted"]) + " ".join(result["vacuous"])
        # caught means: the comparison failed, it says what the defect is, and it names the exchange the defect is in
        exchange = where.split("/")[1]
        good = (not verdict_ok(result)) and needle in text and exchange in text
        print(f"  control {n}  {name}: {'FAILS the comparison (as it must)' if good else 'WAS NOT CAUGHT'}")
        ok &= good
    return ok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--recorded", type=Path, help="the directory record_corpus.py wrote for the target")
    ap.add_argument("--committed", type=Path, default=ORACLE / "corpus")
    ap.add_argument("--register", type=Path, default=ORACLE / "exceptions.yaml")
    ap.add_argument("--schema", type=Path, default=ORACLE / "schema" / "openapi.v1.json")
    ap.add_argument("--scope", required=True)
    ap.add_argument("--target", choices=("plane", "thrust"), default="thrust")
    ap.add_argument("--out", type=Path, help="write the result as JSON")
    ap.add_argument("--show", type=int, default=25, help="how many unaccepted differences to print")
    ap.add_argument("--self-test", action="store_true",
                    help="run the must-fail controls on the committed corpus (no target needed) and exit with their verdict")
    a = ap.parse_args()

    reg = load_register(a.register)
    scope = load_scope(reg, a.scope)
    corpus_cfg = scope.get("corpus")
    if not corpus_cfg or "resources" not in corpus_cfg or "min_exchanges_compared" not in corpus_cfg:
        print(f"scope {a.scope!r} has no `corpus: {{resources, min_exchanges_compared}}`", file=sys.stderr)
        return 2
    regexes = op_regexes(scope_operations(a.schema, scope["tags"]))
    resources = corpus_cfg["resources"]
    try:
        committed_docs = {r: json.load(open(a.committed / f"{r}.json")) for r in resources}
    except FileNotFoundError as e:
        print(f"cannot read {e.filename}", file=sys.stderr)
        return 2

    if a.self_test:
        print(f"[corpus {a.scope}] controls: the comparison must pass on the corpus itself and fail on each defect")
        ok = self_test(committed_docs, resources, regexes, corpus_cfg["min_exchanges_compared"])
        print("CONTROLS " + ("VALID: the comparison passes on the corpus and fails on every defect" if ok
                             else "BROKEN: do not trust a green comparison"))
        return 0 if ok else 1

    if not a.recorded:
        print("--recorded is required (or --self-test)", file=sys.stderr)
        return 2
    try:
        recorded_docs = {r: json.load(open(a.recorded / f"{r}.json")) for r in resources}
    except FileNotFoundError as e:
        print(f"cannot read {e.filename}", file=sys.stderr)
        return 2
    result = compare(committed_docs, recorded_docs, resources, regexes, corpus_entries(reg, a.target),
                     corpus_cfg["min_exchanges_compared"])
    result = {"scope": a.scope, "target": a.target, **result}
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        json.dump(result, open(a.out, "w"), indent=1)
    print_result(f"corpus {a.scope}/{a.target}", result, a.show)
    ok = verdict_ok(result)
    print("VERDICT: " + ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
