#!/usr/bin/env python3
"""Generate the compat acceptance list (F-X13, round 29) as a file, and check it (issue #51).

    python3 -I tools/compat-record/gen-acceptance.py            # write crates/thrust-compat-plane/tests/parity/acceptance.csv
    python3 -I tools/compat-record/gen-acceptance.py --check    # fail unless the committed file is what this would write,
                                                                # and every route in it is a route of the pinned Plane

The acceptance list is the 48-route minimum slice (S01 to S48) and the three rows to open the created work item (X01 to X03), which
are the table in research/0003 section 8.1, plus X04 to X08, five routes the 48 rows lack: one the stock onboarding calls, the
address a project's cover is shown at, the sign-out the user menu calls, and the stickies list and the cover pictures that the first browser
journey's pages call (`crates/thrust-compat-plane/src/users.rs`, `assets.rs`, `auth.rs`, `workspace_load.rs`, `external.rs`, `lib.rs`). The compat replay (`tests/parity.rs`), its fixtures and the oracle recorder
(`record.py`) must each name exactly the rows of this list that are served; `scripts/check-acceptance-list.sh` holds them to it
with `comm -3`. The M2 screens (issue #46) join the list when their rows are filed; until then a route outside it is not a test
obligation.

The `state` column is data and not derived from the fixtures: if it were, "the fixtures match the served rows" would compare
a directory with itself. A row is `pending` until its route is served and recorded, and `served` after it; a row the list adds
(the M2 screens of #46) is `pending` for as long as it has no fixture.

Standard library only: the CI job that runs this has no uv environment.
"""

import argparse
import contextlib
import csv
import io
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
TABLE = REPO / ".agents/plans/contrail/research/0003-compat-plane-inventory.md"
INVENTORY = REPO / "tools/compat-inventory/out-pinned/endpoint-inventory.csv"
OUT = REPO / "crates/thrust-compat-plane/tests/parity/acceptance.csv"

# Not rows of the table, and so named here. Both are routes of the pin (the join below checks it) that the 48-row slice does not name:
# X04, because `finishUserOnboarding` awaits it (users.rs); X05, the address a project's cover is shown at (assets.rs, D-035); X06, the
# sign-out the sidebar's user menu calls (auth.rs, D-036); X07 and X08, the two stubs (D-021) that the first browser journey's home page
# and project form call, the stickies list (workspace_load.rs) and the cover pictures (external.rs).
EXTRA = [
    ("X04", "PATCH", "/api/users/me/onboard/"),
    ("X05", "GET", "/api/assets/v2/static/{}/"),
    ("X06", "POST", "/auth/sign-out/"),
    ("X07", "GET", "/api/workspaces/{}/stickies/"),
    ("X08", "GET", "/api/unsplash/"),
]

# A row is pending while its route is not yet served and recorded; delete it from this set when it is. None is: the asset stage of
# #44 (#124) served S28, S29, S31 and S33, and X05, and this was `{"S28", "S29", "S31", "S33"}` until then.
PENDING = set()
PENDING_STATE = "pending"

HEADER = ["row", "method", "path", "state"]
ROW = re.compile(r"^\|\s*((?:S|X)\d{2})\s*\|[^|]*\|[^|]*\|\s*(GET|POST|PATCH|PUT|DELETE)\s*\|\s*(/[^|\s]*)\s*\|")


def table_rows():
    rows = []
    in_section = False
    for line in TABLE.read_text().splitlines():
        if line.startswith("### 8.1 "):
            in_section = True
        elif line.startswith("### 8.2 "):
            break
        elif in_section:
            m = ROW.match(line)
            if m:
                rows.append(m.groups())
    return rows


def acceptance_rows():
    rows = table_rows() + EXTRA
    ids = [r[0] for r in rows]
    if len(ids) != len(set(ids)):
        sys.exit(f"a row id is given twice: {sorted(i for i in set(ids) if ids.count(i) > 1)}")
    s = [i for i in ids if i.startswith("S")]
    if s != [f"S{n:02d}" for n in range(1, 49)]:
        sys.exit(f"the table's S rows are not S01 to S48 in order: {s}")
    if not {"X01", "X02", "X03"} <= set(ids):
        sys.exit(f"the table has no X01 to X03 rows: {ids}")
    unknown = PENDING - set(ids)
    if unknown:
        sys.exit(f"PENDING names rows that are not in the list: {sorted(unknown)}")
    return [(row, method, path, PENDING_STATE if row in PENDING else "served") for row, method, path in rows]


def render(rows):
    buf = io.StringIO()
    w = csv.writer(buf, lineterminator="\n")
    w.writerow(HEADER)
    w.writerows(rows)
    return buf.getvalue()


def template(path):
    """A path with every parameter, whatever its spelling (`{}`, `<str:slug>`), as `{}`."""
    return re.sub(r"\{[^}]*\}|<[^>]*>", "{}", path)


def inventory_routes():
    with INVENTORY.open(newline="") as f:
        return {(r["method"], template(r["path"])) for r in csv.DictReader(f)}


def check(rows):
    want = render(rows)
    have = OUT.read_text() if OUT.exists() else None
    problems = []
    if have != want:
        problems.append(f"{OUT.relative_to(REPO)} is not what this script writes: run it without --check and commit the result")
    routes = inventory_routes()
    if len(routes) < 100:
        problems.append(f"the inventory has {len(routes)} routes: a join against it would prove nothing")
    for row, method, path, _ in rows:
        if (method, template(path)) not in routes:
            problems.append(f"{row}: {method} {path} is not a route of the pinned Plane (tools/compat-inventory)")
    for p in problems:
        print(f"FAIL: {p}")
    if problems:
        return 1
    print(f"ok: {len(rows)} rows ({sum(1 for r in rows if r[3] == 'served')} served, {len(PENDING)} pending), every route is in the inventory")
    return 0


def self_test():
    """The controls (AGENTS.md testing rule 8): the join can fail, and the spelling of a parameter does not matter."""
    routes = inventory_routes()
    failures = []

    def expect(label, got, want):
        if got != want:
            failures.append(f"{label}: got {got}, wanted {want}")

    expect("a route that is in the inventory joins", ("GET", template("/api/instances/")) in routes, True)
    expect("a route that the pin does not have does not", ("GET", template("/api/workspaces/{}/no-such-thing/")) in routes, False)
    expect("a right path with the wrong method does not", ("DELETE", template("/api/instances/")) in routes, False)
    expect("`{}` and `<str:slug>` are one spelling", template("/a/{}/b/<uuid:id>/"), "/a/{}/b/{}/")
    # a list that has a row whose route is invented, or a file that is stale, is refused
    rows = acceptance_rows()
    invented = rows + [("S99", "GET", "/api/workspaces/{}/no-such-thing/", "served")]
    with contextlib.redirect_stdout(io.StringIO()) as printed:
        refused = check(invented)
    expect("a list with an invented route is refused", refused, 1)
    expect("and it names the invented route", "S99" in printed.getvalue(), True)
    if failures:
        print("\n".join(f"SELF-TEST FAILED: {f}" for f in failures))
        return 1
    print("self-test passed")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        sys.exit(self_test())
    rows = acceptance_rows()
    if args.check:
        sys.exit(check(rows))
    OUT.write_text(render(rows))
    print(f"wrote {OUT.relative_to(REPO)}: {len(rows)} rows")


if __name__ == "__main__":
    main()
