"""Which Worker version is live, and what is its tag? (issue #49, ADR-0110 round 29)

    active-version.py <deployments.json> <versions.json>

The two files are what `wrangler deployments list --name N --json` and `wrangler versions list --name N --json` printed. The deploy tags a
version with the commit sha (`wrangler deploy --tag <sha>`), so the live version's tag is the commit that is running, which is what the
classifier compares the new commit with and what the post-rollback probe expects.

Prints `key=value` lines, ready for $GITHUB_OUTPUT:
  state       none     the Worker has no deployment (it does not exist yet, or was created empty)
              single   one version takes all the traffic
              split    more than one version takes traffic (a gradual deployment this pipeline did not make)
              unknown  the output was not what this reads: no guess is made
  version_id  the live version, when state is single
  tag         its tag, or empty when it has none or is not among the versions listed
  reason      one line

`unknown` and `split` are never turned into `none` or `single`: a deploy that cannot say what it would roll back to must not
roll back by itself (deploy.yml refuses to deploy automatically over either).

UNVERIFIED AGAINST A REAL ACCOUNT. The shapes below are what Cloudflare's API documents (a deployment has `versions: [{version_id,
percentage}]` and `created_on`; a version has `id` and `annotations` with `workers/tag`) and what wrangler is expected to print for
`--json`. The first staging deploy is the check. Until then every deviation lands in `unknown`, which fails safe.
"""

import json
import re
import sys

SAFE = re.compile(r"^[0-9A-Za-z._:/@+=-]+$")


def parse_json_array(text):
    """The JSON array in wrangler's output, tolerating a banner or log lines before or after it."""
    text = text.strip()
    if not text:
        return None, "the output is empty"
    try:
        return json.loads(text), None
    except json.JSONDecodeError:
        pass
    start = text.find("[")
    end = text.rfind("]")
    if start == -1 or end == -1 or end < start:
        return None, "the output has no JSON array in it"
    try:
        return json.loads(text[start : end + 1]), None
    except json.JSONDecodeError as e:
        return None, f"the JSON array in the output does not parse ({e.msg})"


def as_list(parsed, keys):
    """An array, or an object that holds one under one of `keys` (an API envelope)."""
    if isinstance(parsed, list):
        return parsed
    if isinstance(parsed, dict):
        for key in keys:
            value = parsed.get(key)
            if isinstance(value, list):
                return value
        result = parsed.get("result")
        if isinstance(result, (dict, list)):
            return as_list(result, keys)
    return None


def tag_of(version):
    annotations = version.get("annotations")
    if isinstance(annotations, dict) and isinstance(annotations.get("workers/tag"), str):
        return annotations["workers/tag"]
    metadata = version.get("metadata")
    if isinstance(metadata, dict) and isinstance(metadata.get("tag"), str):
        return metadata["tag"]
    if isinstance(version.get("tag"), str):
        return version["tag"]
    return ""


def classify(deployments_text, versions_text):
    parsed, why = parse_json_array(deployments_text)
    if why:
        return {"state": "unknown", "version_id": "", "tag": "", "reason": f"deployments: {why}"}
    deployments = as_list(parsed, ("deployments", "items"))
    if deployments is None:
        return {"state": "unknown", "version_id": "", "tag": "", "reason": "deployments: not a list of deployments"}
    if not deployments:
        return {"state": "none", "version_id": "", "tag": "", "reason": "the Worker has no deployment"}

    if not all(isinstance(d, dict) and isinstance(d.get("created_on"), str) and isinstance(d.get("versions"), list) for d in deployments):
        return {"state": "unknown", "version_id": "", "tag": "", "reason": "deployments: one has no created_on or no versions list"}
    # the order the CLI prints in is not relied on
    live = max(deployments, key=lambda d: d["created_on"])
    taking = [v for v in live["versions"] if isinstance(v, dict) and isinstance(v.get("percentage"), (int, float)) and v["percentage"] > 0]
    if not taking or not all(isinstance(v.get("version_id"), str) and v["version_id"] for v in taking):
        return {"state": "unknown", "version_id": "", "tag": "", "reason": "the latest deployment names no version with traffic"}
    if len(taking) > 1:
        return {"state": "split", "version_id": "", "tag": "", "reason": "the latest deployment splits traffic between versions"}
    only = taking[0]
    if only["percentage"] != 100:
        return {"state": "unknown", "version_id": "", "tag": "", "reason": f"the live version has {only['percentage']}% of traffic, not 100%"}
    version_id = only["version_id"]
    if not SAFE.match(version_id):
        return {"state": "unknown", "version_id": "", "tag": "", "reason": "the live version id has characters an id does not"}

    tag = ""
    note = "its tag was not found among the versions listed"
    vparsed, vwhy = parse_json_array(versions_text)
    versions = as_list(vparsed, ("versions", "items")) if not vwhy else None
    if versions is not None:
        for v in versions:
            if isinstance(v, dict) and v.get("id") == version_id:
                tag = tag_of(v)
                note = "it has no tag" if not tag else "tagged " + tag
                break
    if tag and not SAFE.match(tag):
        return {"state": "unknown", "version_id": "", "tag": "", "reason": "the live version's tag has characters a tag does not"}
    return {"state": "single", "version_id": version_id, "tag": tag, "reason": f"version {version_id} takes all traffic; {note}"}


def main(argv):
    if len(argv) != 3:
        print(__doc__)
        return 2
    try:
        with open(argv[1], encoding="utf-8") as f:
            deployments_text = f.read()
        with open(argv[2], encoding="utf-8") as f:
            versions_text = f.read()
    except OSError as e:
        print(f"active-version: cannot read {e.filename}: {e.strerror}", file=sys.stderr)
        return 2
    out = classify(deployments_text, versions_text)
    for key in ("state", "version_id", "tag", "reason"):
        print(f"{key}={out[key]}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
