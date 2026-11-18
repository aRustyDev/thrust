#!/usr/bin/env python3
"""The configuration of a Preview target (ADR-0111, #195): production's rendered configuration, with a `previews` block derived from it.

    python3 -I scripts/preview-config.py --config <rendered wrangler TOML, or JSON> --profile shared --out <file.json>

A target is a Worker Preview of the production Worker, deployed with `wrangler preview --config <out> --ignore-base-config`.
Previews inherit none of production's bindings: each must be restated under `previews` (Cloudflare's Previews configuration).
Restating them by hand in a second template would drift from the first, so this derives the block from the configuration that
scripts/render-wrangler.sh rendered for production, and writes JSON (which wrangler reads as it reads TOML).

The top level is production's, unchanged: deploying a Preview must not change what production runs. Whether production's
`preview_urls` is on, which a Preview needs for its workers.dev URL, is production's own setting (ADR-0111, Q-0111.1).

The `shared` profile: production's data (ADR-0111, "Targets and their data"). Under `previews`:
  - vars, hyperdrive, r2_buckets, ratelimits, send_email, version_metadata: production's, unchanged. A Preview needs
    version_metadata for the deployment id its smoke compares (research/0016).
  - queues.producers: production's. queues.consumers: none; a consumer never targets a Preview, and production consumes
    what a shared target sends.
  - durable_objects.bindings: production's, each with `script_name` = the production Worker unless it names one already, so
    the rooms are production's rooms running production's room code (measured by the spike, research/0016), not a Preview's
    own namespace, which would snapshot a diverging copy of a page over production's.
  - triggers, routes, migrations, assets, placement: not restated; crons and routes target production only, migrations and
    assets are the Worker's, placement is inherited.
A key the configuration has that this does not know is refused (exit 1), so a binding added to the template cannot be left
out of the targets without a decision. The `isolated` profile takes --resources <trusted JSON>: `pr` (integer), `hyperdrive_id`,
`bucket`, `queue`, `rate_signin`, `rate_v1`, `base_url`, and `csrf`. Bucket and queue names are derived from the PR number;
the identifiers must be supplied by a trusted controller and checked against live resources before deployment. This renderer
does not provision or verify them. Email is disabled, and the room belongs to the Preview. The queue is the events producer
alone: production's dead-letter producer and `THRUST_EVENTS_DLQ` (issue #53) are for the consumer, which a Preview does not have.

Exit 0 written; 1 refused (the reasons on stderr, every one); 2 called wrongly or an unreadable configuration.
"""

import argparse
import copy
import json
import os
import re
import sys
import tomllib
from urllib.parse import urlsplit

# Production's bindings a shared target uses as they are.
MIRRORED = ("vars", "hyperdrive", "r2_buckets", "ratelimits", "send_email", "version_metadata")
# Top-level settings that are production's own: not restated for a Preview.
TOP_ONLY = (
    "name", "main", "compatibility_date", "compatibility_flags", "workers_dev", "preview_urls", "placement", "triggers",
    "routes", "migrations", "assets", "observability", "limits",
)
SPECIAL = ("queues", "durable_objects")
QUEUE_KEYS = ("producers", "consumers")
PROFILES = ("shared", "isolated")


def say(message):
    print(f"preview-config: {message}", file=sys.stderr)


def previews_block(config, profile, resources=None):
    """(block, problems) for this configuration and profile."""
    problems = []
    known = set(MIRRORED) | set(TOP_ONLY) | set(SPECIAL) | {"previews"}
    for key in sorted(set(config) - known):
        problems.append(f"{key!r}: a setting this does not know; decide whether a target restates it, and add it here")
    if "previews" in config:
        problems.append("the configuration already has a `previews` block: render it from the production template, not a target's")
    if not isinstance(config.get("name"), str) or not config["name"]:
        problems.append("the configuration names no Worker (`name`)")
    if profile not in PROFILES:
        problems.append(f"profile {profile!r} is not built; only {list(PROFILES)}")
    if problems:
        return None, problems

    if profile == "isolated":
        return isolated_block(config, resources)
    if resources is not None:
        return None, ["isolated resources cannot be used with the shared profile"]

    block = {k: copy.deepcopy(config[k]) for k in MIRRORED if k in config}
    queues = config.get("queues", {})
    for key in sorted(set(queues) - set(QUEUE_KEYS)):
        problems.append(f"queues.{key}: a queue setting this does not know")
    if queues.get("producers"):
        block["queues"] = {"producers": copy.deepcopy(queues["producers"])}
    durable = config.get("durable_objects", {})
    for key in sorted(set(durable) - {"bindings"}):
        problems.append(f"durable_objects.{key}: a Durable Object setting this does not know")
    if durable.get("bindings"):
        bindings = []
        for b in durable["bindings"]:
            b = copy.deepcopy(b)
            b.setdefault("script_name", config["name"])
            bindings.append(b)
        block["durable_objects"] = {"bindings": bindings}
    if "version_metadata" not in block:
        problems.append("the configuration has no version_metadata binding: a target's smoke compares the deployment id it gives")
    return (None if problems else block), problems


def isolated_block(config, resources):
    """Build an isolated block from trusted resource identifiers; no production data bindings are copied."""
    errors = []
    keys = {"pr", "hyperdrive_id", "bucket", "queue", "rate_signin", "rate_v1", "base_url", "csrf"}
    if not isinstance(resources, dict) or set(resources) != keys:
        return None, ["isolated resources must contain exactly: " + ", ".join(sorted(keys))]
    n = resources["pr"]
    if type(n) is not int or not 1 <= n <= 999999999:
        return None, ["pr must be an integer between 1 and 999999999"]
    for key, expected in (("bucket", f"thrust-pr-{n}-blobs"), ("queue", f"thrust-pr-{n}-events")):
        if resources[key] != expected:
            errors.append(f"{key} must be {expected}")
    hd = resources["hyperdrive_id"]
    if not isinstance(hd, str) or not re.fullmatch(r"[0-9a-f]{32}", hd) or hd == "0" * 32:
        errors.append("hyperdrive_id must be 32 lowercase hex digits and nonzero")
    rate_ids = [resources["rate_signin"], resources["rate_v1"]]
    if any(not isinstance(x, str) or not re.fullmatch(r"[1-9][0-9]*", x) for x in rate_ids) or rate_ids[0] == rate_ids[1]:
        errors.append("rate limit ids must be distinct positive decimal strings")
    try:
        base = urlsplit(resources["base_url"])
        if base.scheme != "https" or not base.hostname or base.username or base.password or base.port or base.path or base.query or base.fragment:
            raise ValueError()
    except (ValueError, TypeError, AttributeError):
        errors.append("base_url must be an HTTPS origin")
    if resources["csrf"] not in ("on", "off"):
        errors.append("csrf must be on or off")

    # The known production shape is explicit. New bindings require an isolation decision.
    expected = {"hyperdrive": ("binding", {"HYPERDRIVE"}), "r2_buckets": ("binding", {"BLOBS"}),
                "ratelimits": ("name", {"RL_SIGNIN", "RL_V1"}), "send_email": ("name", {"EMAIL"})}
    for key, (field, names) in expected.items():
        items = config.get(key)
        if not isinstance(items, list) or any(not isinstance(x, dict) for x in items) or {x.get(field) for x in items} != names or len(items) != len(names):
            errors.append(f"{key} does not have the expected production bindings")
    # Production produces to the events queue and to its dead-letter queue (issue #53: the Worker sends what it sets aside there). A
    # Preview consumes nothing, so it sets nothing aside: it gets the events producer alone, and the dead-letter producer and the
    # variable that names that queue stay production's.
    q = config.get("queues")
    producers = q.get("producers") if isinstance(q, dict) else None
    names = [p.get("binding") if isinstance(p, dict) else None for p in producers] if isinstance(producers, list) else []
    if not isinstance(q, dict) or set(q) - set(QUEUE_KEYS) or sorted(names, key=str) != ["EVENTS", "EVENTS_DLQ"]:
        errors.append("queues does not have the expected EVENTS and EVENTS_DLQ producers")
    rooms = config.get("durable_objects")
    if not isinstance(rooms, dict) or set(rooms) != {"bindings"} or rooms["bindings"] != [{"name": "ROOM", "class_name": "RoomDo"}]:
        errors.append("durable_objects does not have the expected local ROOM binding")
    version = config.get("version_metadata")
    if version != {"binding": "CF_VERSION_METADATA"}:
        errors.append("version_metadata does not have the expected binding")
    if set(config.get("vars", {})) != {"THRUST_CSRF_ENFORCE", "THRUST_EMAIL_FROM", "THRUST_EMAIL_FROM_NAME", "THRUST_EVENTS_DLQ", "THRUST_PUBLIC_BASE_URL"}:
        errors.append("vars does not have the expected production variables")
    if errors:
        return None, errors
    production_queues = {p.get("queue") for p in producers}
    if hd == config["hyperdrive"][0].get("id") or resources["bucket"] == config["r2_buckets"][0].get("bucket_name") or resources["queue"] in production_queues:
        errors.append("isolated data resources must differ from production")
    if set(rate_ids) & {item.get("namespace_id") for item in config["ratelimits"]}:
        errors.append("isolated rate limit ids must differ from production")
    if errors:
        return None, errors
    limits = {item["name"]: item["simple"] for item in config["ratelimits"]}
    return {"vars": {"THRUST_CSRF_ENFORCE": resources["csrf"], "THRUST_PUBLIC_BASE_URL": resources["base_url"]},
            "hyperdrive": [{"binding": "HYPERDRIVE", "id": hd}],
            "r2_buckets": [{"binding": "BLOBS", "bucket_name": resources["bucket"]}],
            "queues": {"producers": [{"binding": "EVENTS", "queue": resources["queue"]}]},
            "ratelimits": [{"name": name, "namespace_id": ns, "simple": copy.deepcopy(limits[name])} for name, ns in zip(("RL_SIGNIN", "RL_V1"), rate_ids)],
            "durable_objects": {"bindings": [{"name": "ROOM", "class_name": "RoomDo"}]},
            "version_metadata": copy.deepcopy(version)}, []


def main(argv):
    parser = argparse.ArgumentParser(prog="preview-config.py")
    parser.add_argument("--config", required=True, help="the rendered production configuration (TOML)")
    parser.add_argument("--profile", required=True)
    parser.add_argument("--resources", help="trusted isolated resources JSON")
    parser.add_argument("--out", required=True)
    args = parser.parse_args(argv[1:])
    try:
        with open(args.config, "rb") as f:
            config = json.load(f) if args.config.endswith(".json") else tomllib.load(f)
        resources = None
        if args.resources:
            with open(args.resources) as f:
                resources = json.load(f)
    except (OSError, tomllib.TOMLDecodeError, json.JSONDecodeError) as e:
        say(f"cannot read {args.config}: {e}")
        return 2
    block, problems = previews_block(config, args.profile, resources)
    if problems:
        for p in problems:
            say(p)
        return 1
    out = copy.deepcopy(config)
    out["previews"] = block
    tmp = args.out + ".tmp"
    with open(tmp, "w") as f:
        json.dump(out, f, indent=2, sort_keys=True)
        f.write("\n")
    os.replace(tmp, args.out)
    say(f"wrote {args.out}: {args.profile} target of {config['name']}, previews restates {sorted(block)}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
