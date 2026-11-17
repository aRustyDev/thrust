#!/usr/bin/env python3
"""Read what `wrangler preview --json` answered for a target, and refuse a Preview that is not the one asked for (ADR-0111, #195).

    python3 -I scripts/preview-result.py <wrangler output> <target config JSON> <label> <preview host suffix>

<wrangler output> is the command's stdout: progress lines, then one JSON object (measured in the spike, research/0016: it starts
with the asset upload's lines). <target config JSON> is what scripts/preview-config.py wrote, whose `previews` block is what the
Preview was asked to bind. Prints `url=<preview URL>` and `deployment=<deployment id>`, one per line, when everything holds:
  - the Preview has the URL the edge will accept for it: `https://<label><suffix>`, a Preview of the production Worker on
    workers.dev (crates/edge/src/route.rs, parse_target), among `.preview.urls`;
  - `.deployment.id` is a deployment id the edge accepts (1 to 64 of lowercase letters, digits and `-`). A Preview's version tag
    is empty, so this id is what its `x-thrust-version` says and what the target's smoke compares (research/0016);
  - every binding the `previews` block declares is in `.deployment.env`, the bindings as Cloudflare applied them;
  - each room binding names the `script_name` it was given: for a shared target, production's Worker, so the rooms are
    production's (research/0016 measured this shape), and not the Preview's own namespace;
  - each variable is the plain text it was given (THRUST_PUBLIC_BASE_URL among them: the shared origin, Q-0111.2).
Other bindings are checked by name only: the spike did not record their shapes in `.deployment.env`.
Exit 0 it holds; 1 it does not (every reason on stderr); 2 called wrongly or an unreadable input.
"""

import json
import re
import sys

DEPLOYMENT = re.compile(r"^[a-z0-9-]{1,64}$")


def say(message):
    print(f"preview-result: {message}", file=sys.stderr)


def answer(raw):
    """The JSON object in wrangler's output: from the first line that starts with `{`."""
    lines = raw.splitlines(keepends=True)
    for i, line in enumerate(lines):
        if line.startswith("{"):
            return json.loads("".join(lines[i:]))
    raise ValueError("no line starts with '{'")


def declared(previews):
    """(binding name, what Cloudflare must show) for each binding the previews block declares."""
    out = []
    for name, value in (previews.get("vars") or {}).items():
        out.append((name, {"type": "plain_text", "text": value}))
    for key, field in (("hyperdrive", "binding"), ("r2_buckets", "binding"), ("ratelimits", "name"), ("send_email", "name")):
        for b in previews.get(key) or []:
            out.append((b[field], None))
    if previews.get("version_metadata"):
        out.append((previews["version_metadata"]["binding"], None))
    for b in (previews.get("queues") or {}).get("producers") or []:
        out.append((b["binding"], None))
    for b in (previews.get("durable_objects") or {}).get("bindings") or []:
        out.append((b["name"], {"script_name": b.get("script_name")}))
    return out


def problems(result, previews, label, suffix):
    found = []
    preview, deployment = result.get("preview") or {}, result.get("deployment") or {}
    want_url = f"https://{label}{suffix}"
    urls = preview.get("urls") or []
    if want_url not in urls:
        found.append(f"the Preview has no URL {want_url} (it has {urls})")
    dep = deployment.get("id")
    if not isinstance(dep, str) or not DEPLOYMENT.match(dep):
        found.append(f"the deployment id {dep!r} is not one the edge accepts")
    env = deployment.get("env") or {}
    for name, must in declared(previews):
        got = env.get(name)
        if not isinstance(got, dict):
            found.append(f"the binding {name} is not in what Cloudflare applied")
            continue
        for field, value in (must or {}).items():
            if got.get(field) != value:
                found.append(f"the binding {name}: {field} is {got.get(field)!r}, it was given {value!r}")
    return found


def main(argv):
    if len(argv) != 5:
        say("usage: preview-result.py <wrangler output> <target config JSON> <label> <preview host suffix>")
        return 2
    _, out_path, config_path, label, suffix = argv
    try:
        with open(out_path) as f:
            result = answer(f.read())
        with open(config_path) as f:
            previews = json.load(f)["previews"]
    except (OSError, ValueError, KeyError) as e:
        say(f"cannot read the inputs: {e}")
        return 2
    found = problems(result, previews, label, suffix)
    for p in found:
        say(p)
    if found:
        return 1
    print(f"url=https://{label}{suffix}")
    print(f"deployment={result['deployment']['id']}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
