"""Compare the deployed Worker's template (wrangler.deploy.toml) with the config every other run uses (wrangler.toml).

    deploy_parity.py <wrangler.toml> <wrangler.deploy.toml>

Exit 0 when they agree on everything that is not a deliberate difference, 1 with the differences listed, 2 when a file is not usable.

The deliberate differences, and nothing else:
  * the probe-only bindings (`SS` Secrets Store, `EMAIL` send_email, `RL` rate limit) are not in the deploy template: only the probe
    routes read them;
  * ids and names are markers in the template (checked by what they are called, not by their value);
  * the queue consumer in the template has a `dead_letter_queue`;
  * wrangler.toml has `[build]` (the deploy job builds nothing), and the template has no `[build]`.

Run from `test-deploy-template.sh`, which also runs it on copies it has broken, to show it can fail.
"""

import sys
import tomllib

PROBE_ONLY = {"SS", "EMAIL", "RL"}


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
    out["var_names"] = sorted(cfg.get("vars", {}))
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
        | {e["name"] for e in cfg.get("send_email", [])} & PROBE_ONLY
        | {r["name"] for r in cfg.get("ratelimits", [])} & PROBE_ONLY
    )
    if deploy:
        out["has_build"] = "build" in cfg
    return out


def main(argv):
    if len(argv) != 3:
        print(__doc__)
        return 2
    base = load(argv[1])
    deploy = load(argv[2])
    a = shape(base, deploy=False)
    b = shape(deploy, deploy=True)
    problems = []

    for key in a:
        if key in ("probe_only_present",):
            continue
        if a[key] != b.get(key):
            problems.append(f"{key}: wrangler.toml has {a[key]!r}, the deploy template has {b.get(key)!r}")
    if b["probe_only_present"]:
        problems.append(f"the deploy template carries probe-only bindings {b['probe_only_present']}")
    if not a["probe_only_present"]:
        problems.append("wrangler.toml has none of the probe-only bindings this comparison expects to leave out: update PROBE_ONLY")
    if b.get("has_build"):
        problems.append("the deploy template has a [build]: the job that holds the credentials builds nothing")
    # every id and name in the template must be a marker, so that a real one is never committed
    for table, key in (("hyperdrive", "id"), ("r2_buckets", "bucket_name")):
        for entry in deploy.get(table, []):
            if not str(entry.get(key, "")).startswith("__"):
                problems.append(f"[[{table}]] {key} is {entry.get(key)!r}, not a marker")
    for c in deploy.get("queues", {}).get("consumers", []):
        if not str(c.get("queue", "")).startswith("__") or not str(c.get("dead_letter_queue", "")).startswith("__"):
            problems.append("the queue consumer's queue and dead_letter_queue must both be markers")
    for p in deploy.get("queues", {}).get("producers", []):
        if not str(p.get("queue", "")).startswith("__"):
            problems.append("the queue producer's queue must be a marker")
    for r in deploy.get("ratelimits", []):
        if not str(r.get("namespace_id", "")).startswith("__"):
            problems.append(f"ratelimit {r.get('name')} namespace_id is {r.get('namespace_id')!r}, not a marker")
    if not str(deploy.get("name", "")).startswith("__"):
        problems.append(f"name is {deploy.get('name')!r}, not a marker")

    if problems:
        print("deploy_parity: the deploy template and wrangler.toml disagree:")
        for p in problems:
            print("  - " + p)
        return 1
    print("deploy_parity: ok (same bindings, same limits, same migrations; probe-only bindings left out; every id and name is a marker)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
