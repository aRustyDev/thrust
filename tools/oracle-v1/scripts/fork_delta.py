#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Summarise the fork delta (files the pin differs from upstream preview) by AREA.

    git diff --numstat <upstream_sha> <pinned_commit> | fork_delta.py   (reads stdin)

Areas are matched in order; the first match wins. The point is to answer, for the oracle:
which fork-only code is in the pinned API, and does any of it touch the v1 wire contract?
"""

import collections
import json
import re
import sys

AREAS = [
    ("oidc-api", r"^apps/api/plane/(authentication/(provider/oauth/oidc|views/(app|space)/oidc|urls|views/__init__|adapter/(error|oauth))|"
                 r"tests/(contract/app/test_(configure_)?instance_oidc|unit/authentication/)|utils/instance_config_variables/core|license/api/views/instance)"),
    ("oidc-frontend", r"^(apps/(admin|web|space)/.*(oidc|oauth)|apps/admin/app/routes\.ts|packages/types/src/instance/(auth|base)\.ts)"),
    ("storage-presigned-put-api", r"^apps/api/plane/(settings/storage|api/views/(asset|issue)|app/views/(asset/v2|issue/attachment)|space/views/asset|tests/unit/settings/test_storage)"),
    ("storage-presigned-put-frontend", r"^(apps/web/services/(file|issue/issue_attachment)|packages/services/src/file/|packages/types/src/file\.ts)"),
    ("ci-governance", r"^(\.github/|\.husky/|\.commitlintrc|CODEOWNERS|SECURITY\.md|release-please-config|\.release-please-manifest|CHANGELOG\.md|\.trivyignore|\.claude/agents/)"),
    ("lockfile-and-manifests", r"^(pnpm-lock\.yaml|package\.json|packages/services/package\.json)$"),
    ("dockerfiles", r"Dockerfile"),
]


def main():
    rows = [l.split("\t") for l in sys.stdin.read().splitlines() if l.strip()]
    agg = collections.OrderedDict((a, {"files": 0, "added": 0, "deleted": 0}) for a, _ in AREAS)
    agg["other"] = {"files": 0, "added": 0, "deleted": 0}
    unmatched = []
    for add, dele, path in rows:
        area = next((a for a, rx in AREAS if re.search(rx, path)), "other")
        if area == "other":
            unmatched.append(path)
        add = 0 if add == "-" else int(add)
        dele = 0 if dele == "-" else int(dele)
        agg[area]["files"] += 1
        agg[area]["added"] += add
        agg[area]["deleted"] += dele
    total = {k: sum(v[k] for v in agg.values()) for k in ("files", "added", "deleted")}
    print(json.dumps({"total": total, "by_area": agg, "unmatched": unmatched}, indent=1))


if __name__ == "__main__":
    sys.exit(main())
