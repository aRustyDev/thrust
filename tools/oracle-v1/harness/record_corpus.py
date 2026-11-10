#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pyyaml"]
# ///
"""Record the golden request/response corpus from the pinned Plane (Option 2 of ADR-0122).

Scripted scenarios drive the REAL API over HTTP with synthetic seeded data. Every exchange is
recorded whatever it returns: success, 400/401/403/404/405/409/415, pagination envelopes and
rate-limit headers. A step whose status differs from `expect` is flagged `surprise` in the
output rather than aborting, because the corpus must show what Plane DOES, not what we hoped.

Outputs
  oracle/corpus/<resource>.json   NORMALISED (oracle/normalise.yaml), committed, diffable
  oracle/.work/corpus-raw/*.json  RAW, never committed; used to prove normalisation is doing work
  oracle/corpus/INDEX.json        counts per resource and per status

Scenario order is part of the contract: unknown ids are numbered by first appearance and
sequence_id is compared verbatim, so do not reorder steps casually.

Usage: record_corpus.py [--base-url http://127.0.0.1:18080] [--ratelimit-url http://127.0.0.1:18081]
"""

import argparse
import http.client
import json
import re
import sys
import time
from pathlib import Path
from urllib.parse import urlencode, urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parent))
from normalise import Normaliser  # noqa: E402

ORACLE = Path(__file__).resolve().parent.parent
TPL = re.compile(r"\{\{([^}]+)\}\}")


def S(name, method, path, **kw):
    return {"name": name, "method": method, "path": path, **kw}


# Short-hands expanded by the template engine below.
WS = "/api/v1/workspaces/{{ws}}"
P = WS + "/projects/{{alpha}}"
PB = WS + "/projects/{{beta}}"

# ---------------------------------------------------------------------------------------------
# Scenarios. `auth` is a manifest token key, or "none", or "bogus". `expect` lists acceptable
# statuses. `cap` stores a value from the JSON response (dotted path) for later {{cap.x}}.
# ---------------------------------------------------------------------------------------------
SCENARIOS = {}

SCENARIOS["errors"] = [
    S("no-api-key", "GET", "/api/v1/users/me/", auth="none", expect=[401]),
    S("bogus-api-key", "GET", "/api/v1/users/me/", auth="bogus", expect=[401]),
    S("expired-api-key", "GET", "/api/v1/users/me/", auth="expired", expect=[401]),
    S("inactive-api-key", "GET", "/api/v1/users/me/", auth="inactive", expect=[401]),
    S("outsider-lists-workspace-projects", "GET", WS + "/projects/", auth="outsider", expect=[403]),
    S("outsider-reads-public-project", "GET", P + "/", auth="outsider", expect=[403, 404]),
    S("member-reads-secret-project", "GET", PB + "/", auth="member", expect=[403, 404]),
    S("guest-creates-project", "POST", WS + "/projects/", auth="guest",
      body={"name": "Guest Project", "identifier": "GST"}, expect=[403]),
    S("guest-creates-work-item", "POST", P + "/work-items/", auth="guest",
      body={"name": "guest item"}, expect=[403]),
    S("unknown-workspace-slug", "GET", "/api/v1/workspaces/no-such-workspace/projects/", auth="owner", expect=[403, 404]),
    S("wrong-method-delete-users-me", "DELETE", "/api/v1/users/me/", auth="owner", expect=[405]),
    S("malformed-uuid-in-path", "GET", WS + "/projects/not-a-uuid/", auth="owner", expect=[404]),
    S("invalid-json-body", "POST", WS + "/projects/", auth="owner", raw_body="{bad json", expect=[400]),
    S("wrong-content-type", "POST", WS + "/projects/", auth="owner",
      raw_body="name=x", content_type="text/plain", expect=[415]),
    S("scalar-json-body", "POST", WS + "/projects/", auth="owner", raw_body='"just a string"', expect=[400]),
    S("unknown-route", "GET", "/api/v1/this-route-does-not-exist/", auth="owner", expect=[404]),
]

SCENARIOS["users"] = [
    S("me", "GET", "/api/v1/users/me/", auth="owner", expect=[200]),
    S("me-as-member", "GET", "/api/v1/users/me/", auth="member", expect=[200]),
]

SCENARIOS["projects"] = [
    S("list", "GET", WS + "/projects/", expect=[200]),
    S("list-per-page-1", "GET", WS + "/projects/", query={"per_page": 1}, expect=[200],
      cap={"next_cursor": "next_cursor"}),
    S("list-second-page", "GET", WS + "/projects/", query={"per_page": 1, "cursor": "{{cap.next_cursor}}"}, expect=[200]),
    S("list-order-by-name", "GET", WS + "/projects/", query={"order_by": "name"}, expect=[200]),
    S("list-fields-projection", "GET", WS + "/projects/", query={"fields": "id,name,identifier"}, expect=[200]),
    S("list-lite", "GET", WS + "/projects-lite/", expect=[200]),
    S("detail", "GET", P + "/", expect=[200]),
    S("detail-by-pk-route", "GET", WS + "/projects/{{alpha}}/", expect=[200]),
    S("detail-unknown", "GET", WS + "/projects/00000000-0000-4000-8000-00000000dead/", expect=[404]),
    S("summary", "GET", P + "/summary/", expect=[200, 404]),
    S("create-scratch", "POST", WS + "/projects/", body={"name": "Oracle Scratch", "identifier": "SCR",
      "description": "scratch project"}, expect=[201], cap={"scratch": "id"}),
    S("create-duplicate-identifier", "POST", WS + "/projects/",
      body={"name": "Oracle Scratch Two", "identifier": "SCR"}, expect=[409]),
    S("create-duplicate-name", "POST", WS + "/projects/",
      body={"name": "Oracle Scratch", "identifier": "SCT"}, expect=[409]),
    S("create-missing-name", "POST", WS + "/projects/", body={"identifier": "NON"}, expect=[400]),
    S("create-identifier-too-long", "POST", WS + "/projects/",
      body={"name": "Long Ident", "identifier": "ABCDEFGHIJKLMNOP"}, expect=[400]),
    S("create-wrong-type", "POST", WS + "/projects/", body={"name": 123, "identifier": "WTY"}, expect=[400]),
    S("patch-scratch", "PATCH", WS + "/projects/{{cap.scratch}}/", body={"description": "patched"}, expect=[200]),
    S("patch-scratch-invalid-network", "PATCH", WS + "/projects/{{cap.scratch}}/", body={"network": 99}, expect=[400, 200]),
    S("patch-unknown", "PATCH", WS + "/projects/00000000-0000-4000-8000-00000000dead/", body={"description": "x"}, expect=[404]),
    S("archive-scratch", "POST", WS + "/projects/{{cap.scratch}}/archive/", expect=[204]),
    S("list-archived-visible", "GET", WS + "/projects/", expect=[200]),
    S("unarchive-scratch", "DELETE", WS + "/projects/{{cap.scratch}}/archive/", expect=[204]),
    S("delete-scratch", "DELETE", WS + "/projects/{{cap.scratch}}/", expect=[204]),
    S("detail-after-delete", "GET", WS + "/projects/{{cap.scratch}}/", expect=[404]),
    # The project's members are soft-deleted by a Celery task (`soft_delete_related_objects`), not by the request, so the pin answers
    # a second DELETE 404 until its worker has run and 403 (no active membership) after. The corpus records the settled answer.
    S("delete-again", "DELETE", WS + "/projects/{{cap.scratch}}/", expect=[404],
      settle={"method": "PATCH", "path": WS + "/projects/{{cap.scratch}}/", "body": {}, "until": 403}),
    S("member-forbidden-delete-public-project", "DELETE", P + "/", auth="member", expect=[403]),
]

SCENARIOS["states"] = [
    S("list", "GET", P + "/states/", expect=[200]),
    S("list-per-page-2", "GET", P + "/states/", query={"per_page": 2}, expect=[200]),
    S("detail", "GET", P + "/states/{{seed.states.alpha:Todo}}/", expect=[200]),
    S("detail-unknown", "GET", P + "/states/00000000-0000-4000-8000-00000000dead/", expect=[404]),
    S("create", "POST", P + "/states/", body={"name": "Oracle Review", "color": "#123456", "group": "started"},
      expect=[201], cap={"state": "id"}),
    S("create-duplicate-name", "POST", P + "/states/",
      body={"name": "Oracle Review", "color": "#123456", "group": "started"}, expect=[409, 400]),
    S("create-invalid-group", "POST", P + "/states/",
      body={"name": "Bad Group", "color": "#123456", "group": "nope"}, expect=[400]),
    S("create-missing-color", "POST", P + "/states/", body={"name": "No Colour"}, expect=[400]),
    S("patch", "PATCH", P + "/states/{{cap.state}}/", body={"name": "Oracle Review 2"}, expect=[200]),
    S("patch-invalid-group", "PATCH", P + "/states/{{cap.state}}/", body={"group": "nope"}, expect=[400]),
    S("delete-default-state", "DELETE", P + "/states/{{seed.states.alpha:Backlog}}/", expect=[400]),
    S("delete", "DELETE", P + "/states/{{cap.state}}/", expect=[204]),
    S("detail-after-delete", "GET", P + "/states/{{cap.state}}/", expect=[404]),
    S("guest-cannot-create", "POST", P + "/states/", auth="guest",
      body={"name": "G", "color": "#000000"}, expect=[403]),
]

SCENARIOS["labels"] = [
    S("list", "GET", P + "/labels/", expect=[200]),
    S("detail", "GET", P + "/labels/{{seed.labels.alpha:bug}}/", expect=[200]),
    S("detail-unknown", "GET", P + "/labels/00000000-0000-4000-8000-00000000dead/", expect=[404]),
    S("create", "POST", P + "/labels/", body={"name": "oracle-scratch", "color": "#ff00ff"},
      expect=[201], cap={"label": "id"}),
    S("create-duplicate", "POST", P + "/labels/", body={"name": "oracle-scratch"}, expect=[409]),
    S("create-missing-name", "POST", P + "/labels/", body={"color": "#000000"}, expect=[400]),
    S("patch", "PATCH", P + "/labels/{{cap.label}}/", body={"color": "#00ff00"}, expect=[200]),
    S("patch-rename-to-existing", "PATCH", P + "/labels/{{cap.label}}/", body={"name": "bug"}, expect=[409, 400]),
    S("delete", "DELETE", P + "/labels/{{cap.label}}/", expect=[204]),
    S("detail-after-delete", "GET", P + "/labels/{{cap.label}}/", expect=[404]),
    S("delete-again", "DELETE", P + "/labels/{{cap.label}}/", expect=[404]),
]

SCENARIOS["work_items"] = [
    S("list", "GET", P + "/work-items/", expect=[200]),
    S("list-legacy-issues-path", "GET", P + "/issues/", expect=[200]),
    S("list-expand-assignees-labels-state", "GET", P + "/work-items/",
      query={"expand": "assignees,labels,state"}, expect=[200]),
    S("list-fields-projection", "GET", P + "/work-items/", query={"fields": "id,name,sequence_id"}, expect=[200]),
    S("list-order-by-sequence", "GET", P + "/work-items/", query={"order_by": "sequence_id"}, expect=[200]),
    S("list-order-by-priority-desc", "GET", P + "/work-items/", query={"order_by": "-priority"}, expect=[200]),
    S("detail", "GET", P + "/work-items/{{seed.issues.i3.id}}/", expect=[200]),
    S("detail-expand", "GET", P + "/work-items/{{seed.issues.i3.id}}/",
      query={"expand": "state,assignees,labels"}, expect=[200]),
    S("detail-legacy-issues-path", "GET", P + "/issues/{{seed.issues.i3.id}}/", expect=[200]),
    S("detail-by-identifier", "GET", WS + "/work-items/ORA-3/", expect=[200]),
    S("detail-by-identifier-unknown", "GET", WS + "/work-items/ORA-9999/", expect=[404]),
    S("detail-unknown", "GET", P + "/work-items/00000000-0000-4000-8000-00000000dead/", expect=[404]),
    S("search", "GET", WS + "/work-items/search/", query={"search": "item"}, expect=[200]),
    S("create-minimal", "POST", P + "/work-items/", body={"name": "Scratch item A"}, expect=[201],
      cap={"wi1": "id"}),
    S("create-full", "POST", P + "/work-items/", body={
        "name": "Scratch item B", "priority": "high", "state": "{{seed.states.alpha:Todo}}",
        "labels": ["{{seed.labels.alpha:bug}}"], "assignees": ["{{seed.users.member.id}}"],
        "description_html": "<p>scratch</p>"}, expect=[201], cap={"wi2": "id"}),
    S("create-legacy-issues-path", "POST", P + "/issues/", body={"name": "Scratch item C"}, expect=[201],
      cap={"wi3": "id"}),
    S("create-missing-name", "POST", P + "/work-items/", body={"priority": "low"}, expect=[400]),
    S("create-invalid-priority", "POST", P + "/work-items/", body={"name": "x", "priority": "super"}, expect=[400]),
    S("create-unknown-state", "POST", P + "/work-items/",
      body={"name": "x", "state": "00000000-0000-4000-8000-00000000dead"}, expect=[400, 404]),
    S("create-external-id", "POST", P + "/work-items/",
      body={"name": "Ext item", "external_source": "oracle", "external_id": "ext-1"}, expect=[201],
      cap={"wi4": "id"}),
    S("create-external-id-duplicate", "POST", P + "/work-items/",
      body={"name": "Ext item again", "external_source": "oracle", "external_id": "ext-1"}, expect=[409]),
    S("patch", "PATCH", P + "/work-items/{{cap.wi1}}/", body={"priority": "urgent", "name": "Scratch item A2"}, expect=[200]),
    S("patch-invalid-priority", "PATCH", P + "/work-items/{{cap.wi1}}/", body={"priority": "super"}, expect=[400]),
    S("patch-unknown", "PATCH", P + "/work-items/00000000-0000-4000-8000-00000000dead/", body={"name": "x"}, expect=[404]),
    S("comments-list", "GET", P + "/work-items/{{seed.issues.i3.id}}/comments/", expect=[200]),
    S("comments-create", "POST", P + "/work-items/{{cap.wi1}}/comments/",
      body={"comment_html": "<p>scratch comment</p>"}, expect=[201], cap={"cm": "id"}),
    S("comments-patch", "PATCH", P + "/work-items/{{cap.wi1}}/comments/{{cap.cm}}/",
      body={"comment_html": "<p>edited</p>"}, expect=[200]),
    S("comments-delete", "DELETE", P + "/work-items/{{cap.wi1}}/comments/{{cap.cm}}/", expect=[204]),
    S("comments-detail-after-delete", "GET", P + "/work-items/{{cap.wi1}}/comments/{{cap.cm}}/", expect=[404]),
    S("links-list", "GET", P + "/work-items/{{seed.issues.i3.id}}/links/", expect=[200]),
    S("links-create", "POST", P + "/work-items/{{cap.wi1}}/links/",
      body={"url": "https://example.test/scratch", "title": "scratch"}, expect=[201], cap={"lk": "id"}),
    S("links-create-invalid-url", "POST", P + "/work-items/{{cap.wi1}}/links/", body={"url": "not a url"}, expect=[400]),
    S("links-delete", "DELETE", P + "/work-items/{{cap.wi1}}/links/{{cap.lk}}/", expect=[204]),
    S("activities-list", "GET", P + "/work-items/{{seed.issues.i3.id}}/activities/", expect=[200]),
    S("activities-detail", "GET", P + "/work-items/{{seed.issues.i3.id}}/activities/{{seed.activities.i3:1}}/", expect=[200]),
    S("relations-create", "POST", P + "/work-items/{{cap.wi1}}/relations/",
      body={"relation_type": "blocking", "issues": ["{{seed.issues.i2.id}}"]}, expect=[201]),
    S("relations-create-invalid-type", "POST", P + "/work-items/{{cap.wi1}}/relations/",
      body={"relation_type": "nonsense", "issues": ["{{seed.issues.i2.id}}"]}, expect=[400]),
    S("delete", "DELETE", P + "/work-items/{{cap.wi1}}/", expect=[204]),
    S("detail-after-delete", "GET", P + "/work-items/{{cap.wi1}}/", expect=[404]),
    S("delete-again", "DELETE", P + "/work-items/{{cap.wi1}}/", expect=[404]),
    S("delete-wi2", "DELETE", P + "/work-items/{{cap.wi2}}/", expect=[204]),
    S("delete-wi3", "DELETE", P + "/work-items/{{cap.wi3}}/", expect=[204]),
    S("delete-wi4", "DELETE", P + "/work-items/{{cap.wi4}}/", expect=[204]),
    S("list-after-deletes", "GET", P + "/work-items/", expect=[200]),
    S("create-after-deletes-sequence", "POST", P + "/work-items/", body={"name": "Scratch item D"}, expect=[201],
      cap={"wi5": "id"}),
    S("delete-wi5", "DELETE", P + "/work-items/{{cap.wi5}}/", expect=[204]),
    # The one operation of the `issues/` family that no exchange above reached: DELETE .../issues/{pk}/. These come last so that no
    # sequence_id or ordinal of an exchange recorded before them moves.
    S("create-for-legacy-delete", "POST", P + "/issues/", body={"name": "Scratch item E"}, expect=[201], cap={"wi6": "id"}),
    S("delete-legacy-issues-path", "DELETE", P + "/issues/{{cap.wi6}}/", expect=[204]),
    S("detail-after-legacy-delete", "GET", P + "/issues/{{cap.wi6}}/", expect=[404]),
]

SCENARIOS["members"] = [
    S("workspace-members", "GET", WS + "/members/", expect=[200]),
    S("project-members", "GET", P + "/members/", expect=[200]),
    S("project-members-alias-route", "GET", P + "/project-members/", expect=[200]),
    S("project-members-lite", "GET", P + "/project-members-lite/", expect=[200]),
    S("project-member-detail", "GET", P + "/members/{{seed.project_members.alpha:member}}/", expect=[200]),
    S("project-member-detail-by-user-id", "GET", P + "/members/{{seed.users.member.id}}/", expect=[404]),
    S("project-member-detail-unknown", "GET", P + "/members/00000000-0000-4000-8000-00000000dead/", expect=[404]),
    S("add-member-to-secret-project", "POST", PB + "/members/",
      body={"member": "{{seed.users.member.id}}", "role": 15}, expect=[201, 200], cap={"pm": "id"}),
    S("add-member-duplicate", "POST", PB + "/members/",
      body={"member": "{{seed.users.member.id}}", "role": 15}, expect=[400, 409]),
    S("add-member-invalid-role", "POST", PB + "/members/",
      body={"member": "{{seed.users.guest.id}}", "role": 77}, expect=[400]),
    S("add-non-workspace-user", "POST", PB + "/members/",
      body={"member": "{{seed.users.outsider.id}}", "role": 15}, expect=[400, 404]),
    S("patch-member-role", "PATCH", PB + "/members/{{cap.pm}}/", body={"role": 20}, expect=[200]),
    S("delete-member", "DELETE", PB + "/members/{{cap.pm}}/", expect=[204]),
    S("delete-member-again", "DELETE", PB + "/members/{{cap.pm}}/", expect=[404]),
    S("guest-cannot-add-member", "POST", P + "/members/", auth="guest",
      body={"member": "{{seed.users.outsider.id}}", "role": 5}, expect=[403]),
]

SCENARIOS["assets"] = [
    S("user-asset-upload", "POST", "/api/v1/assets/user-assets/",
      body={"name": "avatar.png", "type": "image/png", "size": 1024, "entity_type": "USER_AVATAR"},
      expect=[200, 201], cap={"ua": "asset_id"}),
    S("user-asset-upload-missing-name", "POST", "/api/v1/assets/user-assets/",
      body={"type": "image/png", "size": 1024, "entity_type": "USER_AVATAR"}, expect=[400]),
    S("user-asset-upload-bad-entity-type", "POST", "/api/v1/assets/user-assets/",
      body={"name": "a.png", "type": "image/png", "size": 1, "entity_type": "NOPE"}, expect=[400]),
    S("user-asset-upload-bad-mime", "POST", "/api/v1/assets/user-assets/",
      body={"name": "a.exe", "type": "application/x-msdownload", "size": 1, "entity_type": "USER_AVATAR"}, expect=[400]),
    S("user-asset-patch", "PATCH", "/api/v1/assets/user-assets/{{cap.ua}}/", body={"attributes": {}}, expect=[204]),
    S("user-asset-delete", "DELETE", "/api/v1/assets/user-assets/{{cap.ua}}/", expect=[204]),
    S("workspace-asset-upload", "POST", WS + "/assets/",
      body={"name": "doc.png", "type": "image/png", "size": 2048, "project_id": "{{alpha}}"},
      expect=[200, 201], cap={"wa": "asset_id"}),
    S("workspace-asset-upload-oversize", "POST", WS + "/assets/",
      body={"name": "big.png", "type": "image/png", "size": 99999999, "project_id": "{{alpha}}"}, expect=[200, 201, 400]),
    S("workspace-asset-upload-missing-size", "POST", WS + "/assets/", body={"name": "x.png"}, expect=[400]),
    S("workspace-asset-get-not-uploaded", "GET", WS + "/assets/{{cap.wa}}/", expect=[400, 404, 200, 302]),
    S("workspace-asset-mark-uploaded", "PATCH", WS + "/assets/{{cap.wa}}/", body={"is_uploaded": True}, expect=[204]),
    S("workspace-asset-get-uploaded", "GET", WS + "/assets/{{cap.wa}}/", expect=[200, 302, 404]),
    S("workspace-asset-get-unknown", "GET", WS + "/assets/00000000-0000-4000-8000-00000000dead/", expect=[404]),
    S("work-item-attachment-upload", "POST", P + "/work-items/{{seed.issues.i3.id}}/attachments/",
      body={"name": "file.png", "type": "image/png", "size": 4096}, expect=[200, 201], cap={"att": "asset_id"}),
    S("work-item-attachment-upload-missing-name", "POST", P + "/work-items/{{seed.issues.i3.id}}/attachments/",
      body={"size": 4096}, expect=[400]),
    S("work-item-attachments-list", "GET", P + "/work-items/{{seed.issues.i3.id}}/attachments/", expect=[200]),
    S("work-item-attachment-mark-uploaded", "PATCH", P + "/work-items/{{seed.issues.i3.id}}/attachments/{{cap.att}}/",
      body={"is_uploaded": True}, expect=[204, 400]),
    S("work-item-attachment-delete", "DELETE", P + "/work-items/{{seed.issues.i3.id}}/attachments/{{cap.att}}/", expect=[204]),
]

SCENARIOS["pagination"] = [
    S("first-page", "GET", P + "/work-items/", query={"per_page": 3}, expect=[200], cap={"nc": "next_cursor"}),
    S("next-page", "GET", P + "/work-items/", query={"per_page": 3, "cursor": "{{cap.nc}}"}, expect=[200],
      cap={"nc2": "next_cursor", "pc": "prev_cursor"}),
    S("third-page", "GET", P + "/work-items/", query={"per_page": 3, "cursor": "{{cap.nc2}}"}, expect=[200]),
    S("back-to-previous", "GET", P + "/work-items/", query={"per_page": 3, "cursor": "{{cap.pc}}"}, expect=[200]),
    S("per-page-over-max", "GET", P + "/work-items/", query={"per_page": 100000}, expect=[200]),
    S("per-page-zero", "GET", P + "/work-items/", query={"per_page": 0}, expect=[200, 400]),
    S("per-page-negative", "GET", P + "/work-items/", query={"per_page": -1}, expect=[200, 400, 500]),
    S("per-page-not-a-number", "GET", P + "/work-items/", query={"per_page": "abc"}, expect=[200, 400]),
    S("garbage-cursor", "GET", P + "/work-items/", query={"cursor": "garbage"}, expect=[400]),
    S("beyond-last-page", "GET", P + "/work-items/", query={"per_page": 3, "cursor": "3:50:0"}, expect=[200]),
    S("states-first-page", "GET", P + "/states/", query={"per_page": 2}, expect=[200]),
    S("labels-first-page", "GET", P + "/labels/", query={"per_page": 1}, expect=[200]),
]

# Rate limit runs against the OPT-IN low-limit service (5/minute) with its own token.
SCENARIOS["rate_limit"] = [
    S(f"call-{i}", "GET", "/api/v1/users/me/", auth="ratelimit", base="ratelimit",
      expect=[200] if i <= 5 else [429]) for i in range(1, 8)
]

ORDER = ["errors", "users", "projects", "states", "labels", "work_items", "members", "assets", "pagination", "rate_limit"]


def dotted(obj, path):
    for part in path.split("."):
        if isinstance(obj, dict):
            obj = obj.get(part)
        elif isinstance(obj, list) and part.isdigit():
            obj = obj[int(part)]
        else:
            return None
    return obj


class Recorder:
    def __init__(self, base, ratelimit_base, manifest):
        self.base, self.rl_base, self.m = base, ratelimit_base, manifest
        self.ctx = {
            "seed": manifest, "cap": {}, "ws": manifest["workspace"]["slug"],
            "alpha": manifest["projects"]["alpha"]["id"], "beta": manifest["projects"]["beta"]["id"],
        }

    def fmt(self, s):
        def sub(mo):
            key = mo.group(1).strip()
            # `seed.states.alpha:Todo` contains ':' inside a key: split on '.' only
            cur = self.ctx
            for part in key.split("."):
                cur = cur[part] if isinstance(cur, dict) and part in cur else None
                if cur is None:
                    return "MISSING-" + key
            return str(cur)
        return TPL.sub(sub, s)

    def fmt_obj(self, o):
        if isinstance(o, str):
            return self.fmt(o)
        if isinstance(o, list):
            return [self.fmt_obj(x) for x in o]
        if isinstance(o, dict):
            return {k: self.fmt_obj(v) for k, v in o.items()}
        return o

    def token(self, key):
        if key in (None, "owner"):
            return self.m["tokens"]["owner"]
        if key == "none":
            return None
        if key == "bogus":
            return "plane_api_oracle_bogus_00000000000000000000"
        return self.m["tokens"][key]

    def call(self, step):
        base = self.rl_base if step.get("base") == "ratelimit" else self.base
        u = urlsplit(base)
        path = self.fmt(step["path"])
        query = self.fmt_obj(step.get("query") or {})
        url = path + ("?" + urlencode(query) if query else "")
        headers = {"Accept": "application/json"}
        tok = self.token(step.get("auth"))
        if tok:
            headers["X-Api-Key"] = tok
        data = None
        if step.get("body") is not None:
            data = json.dumps(self.fmt_obj(step["body"]))
            headers["Content-Type"] = "application/json"
        elif step.get("raw_body") is not None:
            data = step["raw_body"]
            headers["Content-Type"] = step.get("content_type", "application/json")
        conn = http.client.HTTPConnection(u.hostname, u.port, timeout=60)
        t0 = time.time()
        try:
            conn.request(step["method"], url, body=data, headers=headers)
            r = conn.getresponse()
            raw = r.read()
        finally:
            conn.close()
        ctype = r.getheader("Content-Type") or ""
        body = None
        if raw:
            if "json" in ctype:
                try:
                    body = json.loads(raw)
                except ValueError:
                    body = {"__unparseable__": raw[:300].decode("utf-8", "replace")}
            else:
                body = {"__non_json__": raw[:300].decode("utf-8", "replace")}
        return {
            "name": step["name"], "method": step["method"], "path": path, "query": query,
            "auth": step.get("auth") or "owner", "request_body": self.fmt_obj(step["body"]) if step.get("body") is not None
            else (step.get("raw_body")), "request_content_type": headers.get("Content-Type"),
            "status": r.status, "headers": {k: v for k, v in r.getheaders()}, "body": body,
            "expect": step.get("expect"), "elapsed_s": round(time.time() - t0, 3),
        }

    def settle(self, st):
        """Before a step that depends on work the pin does in a Celery worker, repeat its `settle` probe (not recorded) until it answers
        `until`. A probe that never does is reported and the step is recorded anyway: the corpus shows what the pin did, and the
        comparison then says it differs from the committed one."""
        probe = st["settle"]
        step = {"name": st["name"] + "-settle", "auth": probe.get("auth"), **{k: probe[k] for k in ("method", "path", "body") if k in probe}}
        deadline = time.time() + probe.get("timeout", 30)
        while self.call(step)["status"] != probe["until"]:
            if time.time() > deadline:
                print(f"NOTE {st['name']}: the settle probe never answered {probe['until']} (is the worker running?)", file=sys.stderr)
                return
            time.sleep(0.1)

    def run(self, steps):
        out = []
        for st in steps:
            if st.get("settle"):
                self.settle(st)
            ex = self.call(st)
            for name, path in (st.get("cap") or {}).items():
                self.ctx["cap"][name] = dotted(ex["body"], path)
            out.append(ex)
        return out


def normalise_resource(exchanges, manifest, resource):
    n = Normaliser(manifest=manifest)  # one Normaliser per file: ordinals restart per resource
    res = []
    for e in exchanges:
        res.append({
            "name": e["name"],
            "request": {
                "method": e["method"], "path": n.request_path(e["path"]),
                "query": n.body(e["query"]) if e["query"] else None, "auth": e["auth"],
                "content_type": e["request_content_type"],
                "body": n.body(e["request_body"]) if isinstance(e["request_body"], (dict, list)) else e["request_body"],
            },
            "response": {"status": e["status"], "headers": n.headers(e["headers"], resource),
                         "body": n.body(e["body"])},
            "expect": e["expect"],
            "surprise": e["expect"] is not None and e["status"] not in e["expect"],
        })
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base-url", default="http://127.0.0.1:18080")
    ap.add_argument("--ratelimit-url", default="http://127.0.0.1:18081")
    ap.add_argument("--manifest", type=Path, default=ORACLE / "corpus" / "seed-manifest.json")
    ap.add_argument("--out-dir", type=Path, default=ORACLE / "corpus")
    ap.add_argument("--raw-dir", type=Path, default=ORACLE / ".work" / "corpus-raw")
    ap.add_argument("--only", help="comma-separated resource names")
    a = ap.parse_args()
    manifest = json.load(open(a.manifest))
    pin = dict(l.split("=", 1) for l in open(ORACLE / "PIN").read().splitlines() if "=" in l and not l.startswith("#"))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    a.raw_dir.mkdir(parents=True, exist_ok=True)
    rec = Recorder(a.base_url, a.ratelimit_url, manifest)

    index = {"pinned_commit": pin["pinned_commit"], "resources": {}, "status_counts": {}, "skipped": {}}
    total, surprises = 0, []
    for res in [r for r in ORDER if not a.only or r in a.only.split(",")]:
        if res == "rate_limit":
            u = urlsplit(a.ratelimit_url)
            try:
                http.client.HTTPConnection(u.hostname, u.port, timeout=3).request("GET", "/")
            except OSError as e:
                index["skipped"][res] = f"low-limit service not reachable at {a.ratelimit_url}: {e}"
                print(f"SKIP {res}: {index['skipped'][res]}")
                continue
        ex = rec.run(SCENARIOS[res])
        json.dump(ex, open(a.raw_dir / f"{res}.raw.json", "w"), indent=1)
        norm = normalise_resource(ex, manifest, res)
        doc = {"resource": res, "pinned_commit": pin["pinned_commit"],
               "normalisation": "oracle/normalise.yaml", "exchanges": norm}
        json.dump(doc, open(a.out_dir / f"{res}.json", "w"), indent=1, sort_keys=True)
        open(a.out_dir / f"{res}.json", "a").write("\n")
        counts = {}
        for e in norm:
            s = str(e["response"]["status"])
            counts[s] = counts.get(s, 0) + 1
            index["status_counts"][s] = index["status_counts"].get(s, 0) + 1
            if e["surprise"]:
                surprises.append((res, e["name"], e["response"]["status"], e["expect"]))
        index["resources"][res] = {"exchanges": len(norm), "by_status": dict(sorted(counts.items()))}
        total += len(norm)
        print(f"{res:12} {len(norm):3} exchanges  {dict(sorted(counts.items()))}")
    index["total_exchanges"] = total
    index["surprises"] = [{"resource": r, "step": n, "got": g, "expected": e} for r, n, g, e in surprises]
    index["status_counts"] = dict(sorted(index["status_counts"].items()))
    json.dump(index, open(a.out_dir / "INDEX.json", "w"), indent=1, sort_keys=True)
    open(a.out_dir / "INDEX.json", "a").write("\n")
    print(f"TOTAL {total} exchanges; statuses {index['status_counts']}")
    for r, n, g, e in surprises:
        print(f"  SURPRISE {r}/{n}: got {g}, expected {e}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
