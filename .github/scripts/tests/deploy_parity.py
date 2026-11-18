"""Compare the deployed Worker's template (wrangler.deploy.toml) with the config every other run uses (wrangler.toml).

    deploy_parity.py <wrangler.toml> <wrangler.deploy.toml> [scripts/worker-dev.sh]

Exit 0 when they agree on everything that is not a deliberate difference, 1 with the differences listed, 2 when a file is not usable.

The deliberate differences, and nothing else:
  * the probe-only bindings (`SS` Secrets Store, `RL` rate limit) are not in the deploy template: only the probe routes read them.
    (`EMAIL` send_email is in both: the password reset sends through it, E3. In the template it is restricted to the one sender the
    environment is configured with, `allowed_sender_addresses = ["__EMAIL_FROM__"]`, which this checks);
  * three variables only a deployment sets (`THRUST_EMAIL_FROM`, `THRUST_EMAIL_FROM_NAME`, `THRUST_PUBLIC_BASE_URL`): the template has
    them and wrangler.toml must not, because a development run has no mail service unless a pass gives it one, and the main pass of the
    contract suite holds that the route is dark;
  * ids and names are markers in the template (checked by what they are called, not by their value);
  * the events consumer in the template has a `dead_letter_queue`, which is a marker (wrangler.toml's is a name);
  * wrangler.toml has `[build]` (the deploy job builds nothing), and the template has no `[build]`;
  * the template has `[assets]` (the web app, ADR-0011) and wrangler.toml has none: the contract cells add one to a throwaway copy
    (scripts/worker-dev.sh), so a shared wrangler.toml with one would change what they test. The template's `run_worker_first` must be
    the list worker-dev.sh writes, when worker-dev.sh is given.

Each of the two files must also say the same thing about the queues (ADR-0070, FR-21, issue #53), which `queue_problems` checks in both:
the Worker sends to the dead-letter queue through `EVENTS_DLQ` and reads it back, so the events queue must have a consumer whose
`dead_letter_queue` is that queue, the dead-letter queue must have a consumer of its own with none (a queue cannot be its own
dead-letter queue, and miniflare refuses it), `THRUST_EVENTS_DLQ` must name it (it is how the one queue handler tells the two queues'
batches apart), and the events consumer's `max_retries` must allow the number of deliveries `thrust_ports::MAX_DELIVERIES` says, so that
the queue's own limit only ever backs the Worker's policy up.

Run from `test-deploy-template.sh`, which also runs it on copies it has broken, to show it can fail.
"""

import os
import re
import sys
import tomllib
from pathlib import Path

PROBE_ONLY = {"SS", "RL"}
# Variables a deployment sets and a development run must not: they give the Worker a mail service (E3).
DEPLOY_ONLY_VARS = {"THRUST_EMAIL_FROM", "THRUST_EMAIL_FROM_NAME", "THRUST_PUBLIC_BASE_URL"}
# Where the delivery policy's limit is written: this file is `.github/scripts/tests/deploy_parity.py`, and the repository is three up.
# `DEPLOY_PARITY_PORTS` names another file, which is how `test-deploy-template.sh` shows the check follows the constant and not a number.
PORTS = Path(os.environ.get("DEPLOY_PARITY_PORTS") or Path(__file__).resolve().parents[3] / "crates" / "core" / "ports" / "src" / "lib.rs")


def max_deliveries():
    """`thrust_ports::MAX_DELIVERIES`, read from its source: the one place the number is written."""
    try:
        text = PORTS.read_text(encoding="utf-8")
    except OSError as e:
        print(f"deploy_parity: cannot read {PORTS}: {e}")
        sys.exit(2)
    m = re.search(r"pub const MAX_DELIVERIES: u32 = (\d+);", text)
    if not m:
        print(f"deploy_parity: {PORTS} has no `pub const MAX_DELIVERIES: u32 = N;` to compare the queue's max_retries with")
        sys.exit(2)
    return int(m.group(1))


def queue_problems(what, cfg, *, marker):
    """What is wrong with the events queue and the dead-letter queue behind it in `cfg` (`what` names the file). `marker`: the names
    are markers (the template), not names."""
    out = []
    q = cfg.get("queues", {})
    producers = {p.get("binding"): p.get("queue") for p in q.get("producers", [])}
    by_queue = {c.get("queue"): c for c in q.get("consumers", [])}
    events, dead = producers.get("EVENTS"), producers.get("EVENTS_DLQ")
    if not events or not dead:
        out.append(f"{what}: the EVENTS and EVENTS_DLQ producers are both needed (the Worker puts a dead letter on one and reads it from the other)")
        return out
    if events == dead:
        out.append(f"{what}: the EVENTS and EVENTS_DLQ producers send to the same queue, {events!r}")
    named = cfg.get("vars", {}).get("THRUST_EVENTS_DLQ")
    if named != dead:
        out.append(
            f"{what}: THRUST_EVENTS_DLQ is {named!r}, not {dead!r}, the queue EVENTS_DLQ sends to: the Worker would take one queue's batches for the other's"
        )
    if set(by_queue) != {events, dead}:
        out.append(f"{what}: the queues consumed are {sorted(map(str, by_queue))}, not exactly the events queue and its dead-letter queue")
        return out
    events_consumer, dead_consumer = by_queue[events], by_queue[dead]
    if events_consumer.get("dead_letter_queue") != dead:
        out.append(
            f"{what}: the events consumer's dead_letter_queue is {events_consumer.get('dead_letter_queue')!r}, not {dead!r}, the queue the Worker sets aside to"
        )
    if "dead_letter_queue" in dead_consumer:
        out.append(f"{what}: the dead-letter queue's own consumer has a dead_letter_queue: a queue cannot be its own, and miniflare refuses it")
    need = max_deliveries() - 1
    if events_consumer.get("max_retries", 0) < need:
        out.append(
            f"{what}: the events consumer's max_retries is {events_consumer.get('max_retries')!r}, under {need}: "
            f"the queue would give up on a message before the Worker's policy ({need + 1} deliveries) has"
        )
    if marker:
        for n in (events, dead):
            if not str(n).startswith("__"):
                out.append(f"{what}: queue {n!r} is not a marker")
    return out


def load(path):
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError) as e:
        print(f"deploy_parity: cannot read {path}: {e}")
        sys.exit(2)


def shape(cfg, *, deploy):
    """What the Worker's behaviour depends on, with ids and names reduced to the binding they are for."""
    out = {}
    out["main"] = cfg.get("main")
    out["compatibility_date"] = cfg.get("compatibility_date")
    out["workers_dev"] = cfg.get("workers_dev")
    out["placement"] = cfg.get("placement")
    out["version_metadata"] = cfg.get("version_metadata")
    out["crons"] = cfg.get("triggers", {}).get("crons")
    out["var_names"] = sorted(set(cfg.get("vars", {})) - DEPLOY_ONLY_VARS)
    out["deploy_only_vars_present"] = sorted(set(cfg.get("vars", {})) & DEPLOY_ONLY_VARS)
    out["send_email"] = sorted(e["name"] for e in cfg.get("send_email", []))
    out["hyperdrive"] = sorted(h["binding"] for h in cfg.get("hyperdrive", []))
    out["r2"] = sorted(b["binding"] for b in cfg.get("r2_buckets", []))
    queues = cfg.get("queues", {})
    out["queue_producers"] = sorted(p["binding"] for p in queues.get("producers", []))
    consumers = []
    for c in queues.get("consumers", []):
        consumers.append({k: v for k, v in c.items() if k not in ("queue", "dead_letter_queue")})
    out["queue_consumers"] = consumers
    out["durable_objects"] = sorted((d["name"], d["class_name"]) for d in cfg.get("durable_objects", {}).get("bindings", []))
    out["migrations"] = cfg.get("migrations")
    rl = {r["name"]: r["simple"] for r in cfg.get("ratelimits", []) if r["name"] not in PROBE_ONLY}
    out["ratelimits"] = rl
    # what is in wrangler.toml and must not be in a deployed Worker, or the other way round, is reported separately
    out["probe_only_present"] = sorted(
        {b["binding"] for b in cfg.get("secrets_store_secrets", [])} & PROBE_ONLY
        | {r["name"] for r in cfg.get("ratelimits", [])} & PROBE_ONLY
    )
    if deploy:
        out["has_build"] = "build" in cfg
    return out


def worker_dev_list(path):
    """The `run_worker_first` list scripts/worker-dev.sh writes into the throwaway config of a contract cell."""
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError as e:
        print(f"deploy_parity: cannot read {path}: {e}")
        sys.exit(2)
    m = re.search(r"run_worker_first = (\[[^\]]*\])", text)
    if not m:
        print(f"deploy_parity: {path} has no run_worker_first list to compare with")
        sys.exit(2)
    return tomllib.loads("x = " + m.group(1))["x"]


def main(argv):
    if len(argv) not in (3, 4):
        print(__doc__)
        return 2
    base = load(argv[1])
    deploy = load(argv[2])
    a = shape(base, deploy=False)
    b = shape(deploy, deploy=True)
    problems = []

    for key in a:
        if key in ("probe_only_present", "deploy_only_vars_present"):
            continue
        if a[key] != b.get(key):
            problems.append(f"{key}: wrangler.toml has {a[key]!r}, the deploy template has {b.get(key)!r}")
    if b["probe_only_present"]:
        problems.append(f"the deploy template carries probe-only bindings {b['probe_only_present']}")
    if not a["probe_only_present"]:
        problems.append("wrangler.toml has none of the probe-only bindings this comparison expects to leave out: update PROBE_ONLY")
    if b.get("has_build"):
        problems.append("the deploy template has a [build]: the job that holds the credentials builds nothing")
    # the mail service is a deployment's choice: the template sets its variables, a development config does not
    if a["deploy_only_vars_present"]:
        problems.append(f"wrangler.toml sets {a['deploy_only_vars_present']}: a development run would have a mail service nobody asked for")
    if b["deploy_only_vars_present"] != sorted(DEPLOY_ONLY_VARS):
        problems.append(f"the deploy template sets {b['deploy_only_vars_present']}, not {sorted(DEPLOY_ONLY_VARS)}: the Worker would have no mail service, or a half of one")
    # the binding the reset sends through is restricted to the one sender the environment is configured with
    for e in deploy.get("send_email", []):
        if e.get("name") == "EMAIL" and e.get("allowed_sender_addresses") != ["__EMAIL_FROM__"]:
            problems.append(f"the deploy template's EMAIL binding has allowed_sender_addresses {e.get('allowed_sender_addresses')!r}, not the sender marker: a Worker made to send as another address would be let through")
    if str(deploy.get("vars", {}).get("THRUST_EMAIL_FROM", "")) != "__EMAIL_FROM__":
        problems.append(f"THRUST_EMAIL_FROM is {deploy.get('vars', {}).get('THRUST_EMAIL_FROM')!r} in the deploy template, not the sender marker")
    if str(deploy.get("vars", {}).get("THRUST_PUBLIC_BASE_URL", "")) != "__PUBLIC_BASE_URL__":
        problems.append(f"THRUST_PUBLIC_BASE_URL is {deploy.get('vars', {}).get('THRUST_PUBLIC_BASE_URL')!r} in the deploy template, not the origin marker")
    # every id and name in the template must be a marker, so that a real one is never committed
    for table, key in (("hyperdrive", "id"), ("r2_buckets", "bucket_name")):
        for entry in deploy.get(table, []):
            if not str(entry.get(key, "")).startswith("__"):
                problems.append(f"[[{table}]] {key} is {entry.get(key)!r}, not a marker")
    for c in deploy.get("queues", {}).get("consumers", []):
        # the dead-letter queue's own consumer has none (queue_problems checks that); a consumer that has one names it by a marker
        if not str(c.get("queue", "")).startswith("__") or ("dead_letter_queue" in c and not str(c["dead_letter_queue"]).startswith("__")):
            problems.append("the queue consumer's queue and its dead_letter_queue, where it has one, must be markers")
    for p in deploy.get("queues", {}).get("producers", []):
        if not str(p.get("queue", "")).startswith("__"):
            problems.append("the queue producer's queue must be a marker")
    # the events queue and the dead-letter queue behind it, in each file on its own terms (the structural comparison above holds the two
    # files to each other)
    problems += queue_problems("wrangler.toml", base, marker=False)
    problems += queue_problems("the deploy template", deploy, marker=True)
    for r in deploy.get("ratelimits", []):
        if not str(r.get("namespace_id", "")).startswith("__"):
            problems.append(f"ratelimit {r.get('name')} namespace_id is {r.get('namespace_id')!r}, not a marker")
    if not str(deploy.get("name", "")).startswith("__"):
        problems.append(f"name is {deploy.get('name')!r}, not a marker")

    # the web app: in the template, not in wrangler.toml
    if "assets" in base:
        problems.append("wrangler.toml has an [assets] block: the contract cells add one to a throwaway copy, and a shared one changes what they test")
    assets = deploy.get("assets")
    if not isinstance(assets, dict):
        problems.append("the deploy template has no [assets]: the Worker would not serve the web app")
    else:
        if assets.get("directory") != ".assets":
            problems.append(f"assets.directory is {assets.get('directory')!r}, not '.assets' (where scripts/deploy-worker.sh puts the files)")
        if assets.get("not_found_handling") != "single-page-application":
            problems.append(f"assets.not_found_handling is {assets.get('not_found_handling')!r}, not 'single-page-application'")
        if len(argv) == 4:
            want = worker_dev_list(argv[3])
            if assets.get("run_worker_first") != want:
                problems.append(f"assets.run_worker_first is {assets.get('run_worker_first')!r}, but scripts/worker-dev.sh writes {want!r}")

    if problems:
        print("deploy_parity: the deploy template and wrangler.toml disagree:")
        for p in problems:
            print("  - " + p)
        return 1
    print("deploy_parity: ok (same bindings, same limits, same migrations; probe-only bindings left out; the mail service only where it is deployed, its sender restricted; every id and name is a marker)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
