#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///
"""What the `ci` workflow costs: per job and per run, from GitHub's own run and job records.

    uv run --script .agents/plans/ci-workflows/scripts/ci-baseline.py [--repo OWNER/REPO] [--workflow ci.yml]
        [--per-class 30] [--max-runs 400] [--groups "ci-scripts deps ..."]
    uv run --script .agents/plans/ci-workflows/scripts/ci-baseline.py --self-test

Needs `gh`, authenticated. It prints Markdown (the tables of research/0017-ci-baseline.md) to stdout.

Why not billable minutes: GitHub reports 0 for a public repository (`gh api .../actions/runs/<id>/timing`), so the cost here is
runner-seconds: the sum of `completed_at - started_at` over every job EXECUTION of the run.

An execution is a job name and its `started_at`. GitHub lists a job that passed in attempt 1 again under attempts 2 and 3, with a
new id and its ORIGINAL timestamps (probed 2026-11-16, run 43902442880), so a job's id or its attempt number does not say whether
it ran again; only a new start time does. Every execution counts once, so a re-run's failed attempts (the wasted work) are in the
cost and in the failure share, and a carried-over job is not counted twice.

A run's wall clock is that of its FINAL attempt: `run_started_at` (the attempt's start, which includes any wait for a runner or for
the concurrency group: a push to main queues behind the one before it) to the last completion, split into the wait before the
first job started and the execution span after it. A job's duration is its final entry's (the execution that decided the result).

A run is sorted by what ran, not by what triggered it (a pull request that changes Cargo.lock runs everything too):
  full      every job group of --groups ran
  affected  some did, some were skipped by the planner
  nothing   none did (only `changes` and `gate` ran); counted, not sampled
Only runs that ended `success` or `failure` count: a cancelled run (a superseded pull-request run) measures nothing.
A skipped job is left out of every sum (GitHub stamps it with a start one second after its end).

It fails closed (exit 2) rather than print a table from data it cannot trust: no run to measure, a run with no jobs, a job with
no conclusion, a negative duration, or a job group that --groups does not list (the list has gone stale).
"""

import argparse
import json
import math
import re
import statistics
import subprocess
import sys
import time
from collections import defaultdict
from datetime import datetime, timezone

# The jobs of ci.yml that plan-ci.py can ask for (JOBS in .github/scripts/plan-ci.py). `changes` and `gate` always run.
#
# Each stage of the migration moves a job into a called workflow, whose jobs are `<workflow> / <job>` in the check list, so a job's
# group is what precedes ` / `: `docs` becomes `documentation`, `deny` becomes `security`, `deps` becomes `lint` (as does `lint` itself), and `ci-scripts`, `hermetic`, `pg-adapter` and `mutants` become `test`. A window that holds runs from before a stage needs the names from
# before it (--groups "... docs"), and one from after it these (the default); a run with a name the list lacks stops the script.
DEFAULT_GROUPS = "test lint contract web documentation security"
ALWAYS = ("changes", "gate")
FAILED = ("failure", "timed_out")
SINCE = re.compile(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")


class BaselineError(Exception):
    pass


def when(stamp):
    return datetime.fromisoformat(stamp.replace("Z", "+00:00")).timestamp()


def group_of(name):
    """`contract (native-memory, main) / native-memory main` is a cell of the group `contract`; any other name is its own group."""
    if " / " not in name:
        return name
    head = name.split(" / ")[0]
    return head.split(" (")[0]


def display(name):
    """The row label: a called workflow's job is `<group>: <its own name>`."""
    if " / " not in name:
        return name
    return f"{group_of(name)}: {name.split(' / ', 1)[1]}"


def summarize(run, jobs, groups):
    """One run's measurements, from its run record and the jobs of every attempt. Raises BaselineError on data it cannot trust."""
    rid = run["id"]
    if not jobs:
        raise BaselineError(f"run {rid} has no jobs")
    latest = max(j["run_attempt"] for j in jobs)
    final, executions, seen = [], [], set()  # final: the latest attempt's entries; executions: each (name, start) once
    for j in sorted(jobs, key=lambda j: j["run_attempt"]):
        if j.get("conclusion") is None:
            raise BaselineError(f"run {rid}: job '{j['name']}' has no conclusion (a run that is not over?)")
        if j["conclusion"] == "skipped":
            continue
        started, completed = when(j["started_at"]), when(j["completed_at"])
        if completed < started:
            raise BaselineError(f"run {rid}: job '{j['name']}' ended before it started")
        entry = (display(j["name"]), group_of(j["name"]), completed - started, started, completed, j["conclusion"], j["run_attempt"])
        if j["run_attempt"] == latest:
            final.append(entry)
        if (j["name"], j["started_at"]) not in seen:
            seen.add((j["name"], j["started_at"]))
            executions.append(entry)
    if not final:
        raise BaselineError(f"run {rid}: no job of its latest attempt ran")
    present = {e[1] for e in final} - set(ALWAYS)
    unknown = present - set(groups)
    if unknown:
        raise BaselineError(f"run {rid}: jobs in {sorted(unknown)}, which --groups does not list; update --groups")
    kind = "nothing" if not present else "full" if present == set(groups) else "affected"
    outcomes = defaultdict(lambda: [0, 0])  # label -> [executions, failed executions], over every attempt
    for e in executions:
        outcomes[e[0]][0] += 1
        outcomes[e[0]][1] += e[5] in FAILED
    per_group = defaultdict(float)
    for e in final:
        per_group[e[1]] += e[2]
    fresh = [e for e in executions if e[6] == latest]  # what the final attempt itself ran
    if not fresh:
        raise BaselineError(f"run {rid}: its latest attempt (#{latest}) started no job of its own")
    wait = min(e[3] for e in fresh) - when(run["run_started_at"])
    if wait < -2:
        raise BaselineError(f"run {rid}: a job started {-wait:.0f} s before its attempt did")
    wait = max(wait, 0)
    span = max(e[4] for e in fresh) - min(e[3] for e in fresh)
    return {
        "id": rid, "kind": kind, "event": run["event"], "branch": run.get("head_branch") or "", "attempts": latest,
        "wait": wait, "span": span, "wall": wait + span,
        "runner": sum(e[2] for e in executions), "failed": sum(e[2] for e in executions if e[5] in FAILED),
        "jobs": {e[0]: e[2] for e in final}, "groups": dict(per_group), "outcomes": dict(outcomes),
    }


def stat(values):
    """(n, median, p90 by nearest rank, min, max)."""
    xs = sorted(values)
    return len(xs), statistics.median(xs), xs[math.ceil(0.9 * len(xs)) - 1], xs[0], xs[-1]


def table_for(kind, records):
    out = [f"### {kind}: {len(records)} runs", ""]
    if not records:
        return out + ["none in the window", ""]
    out += ["| per run | median | p90 | min | max |", "|---|---|---|---|---|"]
    for label, key in (("wall clock (s)", "wall"), ("of which waiting before the first job (s)", "wait"),
                       ("execution span (s)", "span"), ("runner-seconds (every execution)", "runner"),
                       ("of which in jobs that failed (s)", "failed")):
        _, med, p90, lo, hi = stat([r[key] for r in records])
        out.append(f"| {label} | {med:.0f} | {p90:.0f} | {lo:.0f} | {hi:.0f} |")
    reran = sum(1 for r in records if r["attempts"] > 1)
    out += ["", f"{reran} of {len(records)} runs needed a re-run (more than one attempt).", ""]
    seconds, outcomes = defaultdict(list), defaultdict(lambda: [0, 0])
    for r in records:
        for name, s in r["jobs"].items():
            seconds[name].append(s)
        for name, (n, f) in r["outcomes"].items():
            outcomes[name][0] += n
            outcomes[name][1] += f
    rows = []
    for name, xs in seconds.items():
        n, med, p90, _, _ = stat(xs)
        ran, failed = outcomes[name]
        rows.append((med, f"| {name} | {n} | {med:.0f} | {p90:.0f} | {failed} of {ran} |"))
    members = defaultdict(set)
    for name in seconds:
        members[name.split(": ")[0] if ": " in name else name].add(name)
    for g, names in sorted(members.items()):
        if len(names) > 1:
            xs = [r["groups"][g] for r in records if g in r["groups"]]
            n, med, p90, _, _ = stat(xs)
            rows.append((med, f"| **{g}, all {len(names)} jobs (sum per run)** | {n} | {med:.0f} | {p90:.0f} | |"))
    out += ["| job | runs | median (s) | p90 (s) | failed of executions, every attempt |", "|---|---|---|---|---|"]
    out += [line for _, line in sorted(rows, key=lambda r: -r[0])]
    return out + [""]


def aggregate(records, scanned, excluded):
    """The Markdown report. Raises BaselineError when there is no run past `changes` to measure."""
    by = defaultdict(list)
    for r in records:
        by[r["kind"]].append(r)
    if not by["full"] and not by["affected"]:
        raise BaselineError("no run past `changes` to measure: there is nothing to baseline")
    stamp = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    lines = [
        f"Collected {stamp}: {scanned} runs scanned, {len(by['full'])} full and {len(by['affected'])} affected-only measured, "
        f"{len(by['nothing'])} planned no job (only `changes` and `gate`), excluded: "
        + (", ".join(f"{n} {c}" for c, n in sorted(excluded.items())) or "none") + ".", "",
    ]
    for kind in ("full", "affected"):
        lines += table_for(kind, by[kind])
    if by["nothing"]:
        _, med, p90, _, _ = stat([r["wall"] for r in by["nothing"]])
        _, wmed, wp90, _, _ = stat([r["wait"] for r in by["nothing"]])
        lines += [f"### nothing: {len(by['nothing'])} runs", "",
                  f"wall clock median {med:.0f} s, p90 {p90:.0f} s; of which waiting before the first job median {wmed:.0f} s, "
                  f"p90 {wp90:.0f} s.", ""]
    lines += ["### runs measured", ""]
    for kind in ("full", "affected"):
        lines.append(f"- {kind}: " + ", ".join(f"{r['id']} ({r['event']})" for r in by[kind]))
    return "\n".join(lines)


def gh_json(path, paginate=False, run=subprocess.run, sleep=time.sleep):
    """The JSON pages of a GitHub API path. A server error (HTTP 5xx) is tried three times, 2 s then 4 s apart; anything else, or a
    third failure, stops the run: a table is never printed from a scan that skipped a run."""
    cmd = ["gh", "api"] + (["--paginate"] if paginate else []) + [path]
    for attempt in range(3):
        done = run(cmd, capture_output=True, text=True)
        if done.returncode == 0:
            break
        if "HTTP 5" not in done.stderr or attempt == 2:
            raise BaselineError(f"gh api {path} failed: {done.stderr.strip()}")
        sleep(2 * (attempt + 1))
    decoder, pos, pages = json.JSONDecoder(), 0, []
    while True:
        while pos < len(done.stdout) and done.stdout[pos].isspace():
            pos += 1
        if pos >= len(done.stdout):
            return pages
        page, pos = decoder.raw_decode(done.stdout, pos)
        pages.append(page)


def check_since(since):
    if since is not None and not SINCE.match(since):
        raise BaselineError(f"--since '{since}' is not a UTC time like 2026-11-16T15:51:24Z")


def collect(repo, workflow, groups, per_class, max_runs, since=None):
    check_since(since)
    records, excluded, scanned, page = [], defaultdict(int), 0, 1
    while scanned < max_runs:
        counts = defaultdict(int)
        for r in records:
            counts[r["kind"]] += 1
        if counts["full"] >= per_class and counts["affected"] >= per_class:
            break
        path = f"repos/{repo}/actions/workflows/{workflow}/runs?per_page=100&status=completed&page={page}"
        runs = gh_json(path)[0]["workflow_runs"]
        if not runs:
            break
        for run in runs:
            if scanned >= max_runs:
                break
            if since and run["created_at"] < since:  # newest first, so every run after this one is older still
                return records, scanned, excluded
            scanned += 1
            if run["conclusion"] not in ("success", "failure"):
                excluded[run["conclusion"]] += 1
                continue
            # Pages of 30, not 100: for a run with three attempts (108 jobs) GitHub answered every `per_page=100` request with
            # HTTP 502 and `per_page=30` with the jobs (probed 2026-11-16, run 43902442880).
            jobs = [j for p in gh_json(f"repos/{repo}/actions/runs/{run['id']}/jobs?filter=all&per_page=30", True) for j in p["jobs"]]
            record = summarize(run, jobs, groups)
            if record["kind"] == "nothing" or counts[record["kind"]] < per_class:
                records.append(record)
                counts[record["kind"]] += 1
        page += 1
    return records, scanned, excluded


def self_test():
    """The arithmetic on a hand-worked example, and each refusal: a check that cannot fail is not evidence."""
    ok = bad = 0

    def check(label, cond):
        nonlocal ok, bad
        ok, bad = (ok + 1, bad) if cond else (ok, bad + 1)
        print(("ok   " if cond else "BAD  ") + label)

    def refused(label, fragment, fn):
        try:
            fn()
        except BaselineError as e:
            check(f"{label} (said: {e})", fragment in str(e))
        else:
            check(f"{label}: it was NOT refused", False)

    def iso(sec):
        return datetime.fromtimestamp(1_700_000_000 + sec, timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

    def job(name, start, end, conclusion="success", attempt=1):
        return {"name": name, "run_attempt": attempt, "conclusion": conclusion, "started_at": iso(start), "completed_at": iso(end)}

    def run(rid, event="pull_request", started=0):
        return {"id": rid, "event": event, "head_branch": "x", "run_started_at": iso(started)}

    groups = ("a", "b")
    # three full runs: a takes 10, 20, 30 s; b takes 30, 40, 50 s; changes and gate take 5 s each, in series around them
    recs = []
    for i, (da, db) in enumerate([(10, 30), (20, 40), (30, 50)]):
        jobs = [job("changes", 0, 5), job("a", 5, 5 + da), job("b", 5, 5 + db), job("gate", 5 + db, 10 + db)]
        recs.append(summarize(run(i), jobs, groups))
    check("every run with all groups is full", all(r["kind"] == "full" for r in recs))
    check("wall clocks are 40, 50, 60", [r["wall"] for r in recs] == [40, 50, 60])
    check("runner-seconds are 50, 70, 90", [r["runner"] for r in recs] == [50, 70, 90])
    check("stat of 10,20,30: n, median, p90 (nearest rank), min, max", stat([10, 20, 30]) == (3, 20, 30, 10, 30))
    check("stat of one value", stat([7]) == (1, 7, 7, 7, 7))
    report = aggregate(recs, 3, {})
    check("the report has the job row for b: median 40, p90 50", "| b | 3 | 40 | 50 | 0 of 3 |" in report)
    check("the report has the full run's wall clock row: median 50, p90 60", "| wall clock (s) | 50 | 60 | 40 | 60 |" in report)

    # a skipped job is stamped one second after its end: it must not count, and must not be an error
    affected = summarize(run(10), [job("changes", 0, 5), job("a", 6, 5, "skipped"), job("b", 5, 15), job("gate", 15, 20)], groups)
    check("one group skipped: affected, and the skipped job adds nothing", affected["kind"] == "affected" and affected["runner"] == 20)
    nothing = summarize(run(11), [job("changes", 0, 5), job("a", 6, 5, "skipped"), job("b", 6, 5, "skipped"), job("gate", 5, 9)], groups)
    check("no group ran: nothing", nothing["kind"] == "nothing")

    # a re-run in which every job ran again (new start times): the cost is both attempts', the duration is the final one's
    rerun = summarize(run(12, started=40), [job("changes", 0, 5), job("b", 5, 35, "failure"), job("a", 5, 15),
                                            job("changes", 40, 45, attempt=2), job("a", 45, 55, attempt=2), job("b", 45, 73, attempt=2)],
                      groups)
    check("a re-run reports attempt 2", rerun["attempts"] == 2)
    check("the re-run's duration of b is the final attempt's (28 s)", rerun["jobs"]["b"] == 28)
    check("the re-run's failure of b is still counted (2 executions, 1 failed)", rerun["outcomes"]["b"] == [2, 1])
    check("the cost is both attempts' (5+30+10 and 5+10+28), 30 s of it in a failed job", rerun["runner"] == 88 and rerun["failed"] == 30)
    check("the wall clock is the final attempt's: 40 to 73", rerun["wall"] == 33 and rerun["wait"] == 0 and rerun["span"] == 33)

    # a re-run of only the failed job: GitHub lists `a` again under attempt 2, new id, ORIGINAL times. It ran once.
    carried = summarize(run(13, started=40), [job("changes", 0, 5), job("a", 5, 15), job("b", 5, 35, "failure"),
                                              job("changes", 40, 45, attempt=2), job("a", 5, 15, attempt=2), job("b", 45, 60, attempt=2)],
                        groups)
    check("a carried-over job is one execution, not two", carried["outcomes"]["a"] == [1, 0] and carried["kind"] == "full")
    check("its duration is the original one, and the cost counts it once (5+10+30+5+15)", carried["jobs"]["a"] == 10 and carried["runner"] == 65)
    check("the wall clock is what attempt 2 ran itself: 40 to 60", carried["wall"] == 20)

    # the time before the first job: a run that waited 7 s for a runner or a concurrency group
    waited = summarize(run(14), [job("changes", 7, 12), job("a", 12, 22), job("b", 12, 32), job("gate", 32, 37)], groups)
    check("waiting is split from execution: 7 + 30 = 37", waited["wait"] == 7 and waited["span"] == 30 and waited["wall"] == 37)
    refused("a job that started long before its attempt did", "before its attempt did",
            lambda: summarize(run(15, started=100), [job("changes", 0, 5), job("a", 5, 9)], groups))

    # the refusals
    refused("a run with no jobs", "has no jobs", lambda: summarize(run(20), [], groups))
    refused("a job with no conclusion", "has no conclusion", lambda: summarize(run(21), [job("changes", 0, 5), job("a", 5, 9, None)], groups))
    refused("a job that ends before it starts", "ended before it started", lambda: summarize(run(22), [job("changes", 0, 5), job("a", 9, 5)], groups))
    refused("a job group --groups does not list", "which --groups does not list", lambda: summarize(run(23), [job("changes", 0, 5), job("zzz", 5, 9)], groups))
    refused("a run where nothing ran at all", "no job of its latest attempt ran", lambda: summarize(run(24), [job("changes", 6, 5, "skipped")], groups))
    refused("an empty list of runs", "nothing to baseline", lambda: aggregate([], 0, {}))
    refused("only runs that planned no job", "nothing to baseline", lambda: aggregate([nothing], 1, {}))

    # the retry: a 5xx is tried again (and only 5xx, and only so often), so one flaky request does not end a long scan
    class Done:
        def __init__(self, code, err="", out="{}"):
            self.returncode, self.stderr, self.stdout = code, err, out

    def scripted(*answers):
        calls = []

        def fake(cmd, **_):
            calls.append(cmd)
            return answers[min(len(calls), len(answers)) - 1]

        return fake, calls

    waits = []
    fake, calls = scripted(Done(1, "gh: Server Error (HTTP 502)"), Done(1, "gh: Server Error (HTTP 502)"), Done(0, out='{"a": 1}'))
    check("two 502s then success: the page, after 3 calls and 2 waits",
          gh_json("p", run=fake, sleep=waits.append) == [{"a": 1}] and len(calls) == 3 and waits == [2, 4])
    fake, calls = scripted(Done(1, "gh: Server Error (HTTP 502)"))
    refused("three 502s in a row", "HTTP 502", lambda: gh_json("p", run=fake, sleep=lambda s: None))
    check("three 502s in a row were three calls, no more", len(calls) == 3)
    fake, calls = scripted(Done(1, "gh: Not Found (HTTP 404)"))
    refused("a 404", "HTTP 404", lambda: gh_json("p", run=fake, sleep=lambda s: None))
    check("a 404 was not tried again", len(calls) == 1)

    # --since: a malformed time is refused (a typo must not mean "no limit"), a good one is accepted
    check("a UTC time is accepted", check_since("2026-11-16T15:51:24Z") is None and check_since(None) is None)
    for bad_since in ("2026-11-16", "2026-11-16T15:51:24+00:00", "yesterday", ""):
        refused(f"--since '{bad_since}'", "is not a UTC time", lambda s=bad_since: check_since(s))

    # grouping and labels, as they appear in a real run
    cell = "contract (worker-cf, e2e) / worker-cf e2e"
    check("a cell's group is its caller job", group_of(cell) == "contract" and group_of("hermetic") == "hermetic")
    check("a cell's label", display(cell) == "contract: worker-cf e2e" and display("lint") == "lint")

    print(f"passed={ok} bad={bad}")
    return bad == 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--repo", default="aRustyDev/thrust")
    ap.add_argument("--workflow", default="ci.yml")
    ap.add_argument("--per-class", type=int, default=30, help="the most runs measured of each of full and affected (default 30)")
    ap.add_argument("--max-runs", type=int, default=400, help="the most runs scanned, newest first (default 400)")
    ap.add_argument("--groups", default=DEFAULT_GROUPS, help="the job groups a full run runs, space separated")
    ap.add_argument("--since", help="only runs created at or after this UTC time (2026-11-16T15:51:24Z): the moment ci.yml's job set "
                                    "last changed, since an older run has other jobs and is not comparable")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        sys.exit(0 if self_test() else 1)
    try:
        records, scanned, excluded = collect(args.repo, args.workflow, args.groups.split(), args.per_class, args.max_runs, args.since)
        print(aggregate(records, scanned, excluded))
    except BaselineError as e:
        print(f"ci-baseline: {e}", file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()
