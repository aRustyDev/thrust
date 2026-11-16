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
out of the targets without a decision. `isolated` (a target's own data, stage F of #195) is not built yet and is refused.

Exit 0 written; 1 refused (the reasons on stderr, every one); 2 called wrongly or an unreadable configuration.
"""

import argparse
import copy
import json
import os
import sys
import tomllib

# Production's bindings a shared target uses as they are.
MIRRORED = ("vars", "hyperdrive", "r2_buckets", "ratelimits", "send_email", "version_metadata")
# Top-level settings that are production's own: not restated for a Preview.
TOP_ONLY = (
    "name", "main", "compatibility_date", "compatibility_flags", "workers_dev", "preview_urls", "placement", "triggers",
    "routes", "migrations", "assets", "observability", "limits",
)
SPECIAL = ("queues", "durable_objects")
QUEUE_KEYS = ("producers", "consumers")
PROFILES = ("shared",)


def say(message):
    print(f"preview-config: {message}", file=sys.stderr)


def previews_block(config, profile):
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
        problems.append(f"profile {profile!r} is not built; only {list(PROFILES)} (isolated targets come with stage F of #195)")
    if problems:
        return None, problems

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


def main(argv):
    parser = argparse.ArgumentParser(prog="preview-config.py")
    parser.add_argument("--config", required=True, help="the rendered production configuration (TOML)")
    parser.add_argument("--profile", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args(argv[1:])
    try:
        with open(args.config, "rb") as f:
            config = json.load(f) if args.config.endswith(".json") else tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError, json.JSONDecodeError) as e:
        say(f"cannot read {args.config}: {e}")
        return 2
    block, problems = previews_block(config, args.profile)
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
