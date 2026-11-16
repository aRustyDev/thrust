"""scripts/preview-config.py: the configuration of a shared Preview target, derived from production's (ADR-0111, #195).

    test_preview_config.py [path to a copy of preview-config.py]

1. The real template: crates/worker/wrangler.deploy.toml rendered by scripts/render-wrangler.sh with production-shaped values,
   then derived. The top level is production's, byte for byte as a value; `previews` restates every binding as the model
   says; no consumer, cron, route, migration or asset is restated; every room binding names the production Worker.
2. Generated configurations (seeded, 400 of them): every subset of the mirrored bindings, queues with and without producers
   and consumers, room bindings with and without a script_name of their own, every subset of the top-level-only settings.
   The output equals the model's, which is written here and not taken from the script.
3. Refusals, each with nothing written: every binding kind the script does not know, a configuration that already has a
   `previews` block, no name, no version_metadata, an unknown queue or Durable Object setting, the `isolated` profile;
   and an unreadable file is exit 2.
A path argument points it at a deliberately broken copy, to show this test fails.
"""

import copy
import json
import os
import random
import subprocess
import sys
import tempfile
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
SCRIPT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "scripts", "preview-config.py")
MIRRORED = ["vars", "hyperdrive", "r2_buckets", "ratelimits", "send_email", "version_metadata"]
TOP_ONLY = ["main", "compatibility_date", "compatibility_flags", "workers_dev", "preview_urls", "placement", "triggers",
            "routes", "migrations", "assets", "observability", "limits"]

passed = failed = 0
work = tempfile.mkdtemp(prefix="preview-config-test-")


def check(label, got, want):
    global passed, failed
    if got == want:
        passed += 1
    else:
        failed += 1
        print(f"BAD  {label}: got {got!r}, wanted {want!r}")


def model(cfg):
    """What a shared target restates, written from ADR-0111 and Cloudflare's Previews configuration, not from the script."""
    out = {}
    for key in MIRRORED:
        if key in cfg:
            out[key] = cfg[key]
    producers = cfg.get("queues", {}).get("producers")
    if producers:
        out["queues"] = {"producers": producers}
    rooms = cfg.get("durable_objects", {}).get("bindings")
    if rooms:
        out["durable_objects"] = {"bindings": [{**b, "script_name": b.get("script_name", cfg["name"])} for b in rooms]}
    return out


def derive(cfg_path, profile="shared"):
    out = os.path.join(work, "out.json")
    if os.path.exists(out):
        os.remove(out)
    p = subprocess.run([sys.executable, "-I", SCRIPT, "--config", cfg_path, "--profile", profile, "--out", out],
                       capture_output=True, text=True)
    produced = None
    if os.path.exists(out):
        with open(out) as f:
            produced = json.load(f)
    return p.returncode, p.stdout + p.stderr, produced


def write_json(cfg, name="cfg.json"):
    path = os.path.join(work, name)
    with open(path, "w") as f:
        json.dump(cfg, f)
    return path


# ---- 1. the real template ------------------------------------------------------------------------------------------------
PROD_ENV = {
    "PATH": os.environ.get("PATH", ""),
    "THRUST_WORKER_NAME": "thrust-production", "THRUST_HYPERDRIVE_ID": "0123456789abcdef0123456789abcdef",
    "THRUST_R2_BUCKET": "thrust-production-blobs", "THRUST_QUEUE": "thrust-production-events",
    "THRUST_QUEUE_DLQ": "thrust-production-events-dlq", "THRUST_RL_SIGNIN_ID": "93002", "THRUST_RL_V1_ID": "93003",
    "THRUST_CSRF_ENFORCE": "on", "THRUST_EMAIL_FROM": "no-reply@notify.thrust-staging.dev",
    "THRUST_BASE_URL": "https://thrust.example-shared.dev",
}
rendered = os.path.join(work, "rendered.toml")
r = subprocess.run(["sh", os.path.join(ROOT, "scripts", "render-wrangler.sh"), "--out", rendered], capture_output=True, text=True,
                   env=PROD_ENV)
check("the production template renders", (r.returncode, r.stderr[-300:] if r.returncode else ""), (0, ""))
with open(rendered, "rb") as f:
    prod = tomllib.load(f)
rc, out, got = derive(rendered)
check("the real template derives", rc, 0)
if got:
    top = {k: v for k, v in got.items() if k != "previews"}
    check("the top level is production's, unchanged", top, prod)
    check("previews restates what the model says", got["previews"], model(prod))
    check("previews restates the bindings the template has", sorted(got["previews"]),
          sorted(["vars", "hyperdrive", "r2_buckets", "ratelimits", "send_email", "version_metadata", "queues", "durable_objects"]))
    check("no consumer for a target", "consumers" in got["previews"].get("queues", {}), False)
    for key in ("triggers", "routes", "migrations", "assets", "placement"):
        check(f"{key} is not restated", key in got["previews"], False)
    check("every room binding names the production Worker",
          {b.get("script_name") for b in got["previews"]["durable_objects"]["bindings"]}, {"thrust-production"})
    check("the public base URL is the shared one, production's", got["previews"]["vars"]["THRUST_PUBLIC_BASE_URL"],
          "https://thrust.example-shared.dev")
    check("the production template has a consumer and a cron (so dropping them was tested)",
          (bool(prod.get("queues", {}).get("consumers")), bool(prod.get("triggers"))), (True, True))

# ---- 2. generated configurations ------------------------------------------------------------------------------------------
rng = random.Random(195)


def generated():
    cfg = {"name": rng.choice(["thrust-production", "w", "thrust-x-1"])}
    for key in MIRRORED:
        if rng.random() < 0.6:
            cfg[key] = {"vars": {"A": str(rng.random())}, "hyperdrive": [{"binding": "HD", "id": "ab" * 16}],
                        "r2_buckets": [{"binding": "B", "bucket_name": f"b{rng.randint(0, 9)}"}],
                        "ratelimits": [{"name": "RL", "namespace_id": str(rng.randint(1, 99999)), "simple": {"limit": 10, "period": 60}}],
                        "send_email": [{"name": "EMAIL"}], "version_metadata": {"binding": "CF_VERSION_METADATA"}}[key]
    if rng.random() < 0.7:
        q = {}
        if rng.random() < 0.6:
            q["producers"] = [{"binding": "EVENTS", "queue": f"q{rng.randint(0, 9)}"}]
        if rng.random() < 0.6:
            q["consumers"] = [{"queue": "q", "max_retries": 3}]
        cfg["queues"] = q
    if rng.random() < 0.7:
        rooms = []
        for i in range(rng.randint(0, 3)):
            b = {"name": f"R{i}", "class_name": f"C{i}"}
            if rng.random() < 0.3:
                b["script_name"] = "another-worker"
            rooms.append(b)
        cfg["durable_objects"] = {"bindings": rooms}
    for key in TOP_ONLY:
        if rng.random() < 0.4:
            cfg[key] = {"triggers": {"crons": ["* * * * *"]}, "routes": [{"pattern": "x.example/*"}],
                        "migrations": [{"tag": "v1"}], "assets": {"directory": ".assets"}}.get(key, f"value-of-{key}")
    return cfg


seen = {"with version_metadata": 0, "refused for its absence": 0, "consumers dropped": 0, "a room with its own script_name": 0}
for i in range(400):
    cfg = generated()
    rc, out, got = derive(write_json(cfg))
    if "version_metadata" not in cfg:
        seen["refused for its absence"] += 1
        check(f"generated {i}: no version_metadata is refused, nothing written", (rc, got), (1, None))
        continue
    seen["with version_metadata"] += 1
    seen["consumers dropped"] += bool(cfg.get("queues", {}).get("consumers"))
    seen["a room with its own script_name"] += any("script_name" in b for b in cfg.get("durable_objects", {}).get("bindings", []))
    check(f"generated {i}: exit 0", rc, 0)
    if got:
        check(f"generated {i}: the top level is the input's", {k: v for k, v in got.items() if k != "previews"}, cfg)
        check(f"generated {i}: previews is the model's", got["previews"], model(cfg))
# rule 8: the generator reached every class it is meant to
for what, n in seen.items():
    check(f"the generator produced cases {what}", n > 10, True)

# ---- 3. refusals ----------------------------------------------------------------------------------------------------------
base = {"name": "thrust-production", "version_metadata": {"binding": "CF_VERSION_METADATA"}}
for unknown in ("kv_namespaces", "d1_databases", "services", "ai", "secrets", "vectorize", "workflows", "analytics_engine_datasets"):
    rc, out, got = derive(write_json({**base, unknown: [{"binding": "X"}]}))
    check(f"refused: an unknown binding kind {unknown}, named, nothing written", (rc, unknown in out, got), (1, True, None))
for label, cfg, needle in (
    ("a configuration with a previews block", {**base, "previews": {}}, "already has a `previews` block"),
    ("no name", {k: v for k, v in base.items() if k != "name"}, "names no Worker"),
    ("an unknown queue setting", {**base, "queues": {"producers": [], "dlq": []}}, "queues.dlq"),
    ("an unknown Durable Object setting", {**base, "durable_objects": {"bindings": [], "classes": []}}, "durable_objects.classes"),
):
    rc, out, got = derive(write_json(cfg))
    check(f"refused: {label}, named, nothing written", (rc, needle in out, got), (1, True, None))
rc, out, got = derive(write_json(base), profile="isolated")
check("refused: the isolated profile (stage F), nothing written", (rc, "isolated" in out, got), (1, True, None))
rc, out, got = derive(os.path.join(work, "missing.toml"))
check("an unreadable configuration: exit 2", (rc, got), (2, None))

print(f"test_preview_config: {passed} passed, {failed} failed")
sys.exit(1 if failed else 0)
