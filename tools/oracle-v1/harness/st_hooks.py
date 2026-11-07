"""Schemathesis hooks for the Plane v1 oracle: aim generated requests at the SEEDED entities.

Without this, schemathesis invents a random workspace slug and uuids, so ~116 of 130 operations
only ever see 401/403/404 and the response schemas of the real success paths are never exercised
(measured: "Authentication failed: 116 operations returned only 401/403 responses"). That would
make a green run meaningless. With the seeded ids, GET/POST/PATCH reach the real handlers.

Loaded with:  SCHEMATHESIS_HOOKS=st_hooks  PYTHONPATH=oracle/harness
The manifest path comes from ORACLE_MANIFEST (default oracle/corpus/seed-manifest.json).

Ids are keyed off the path TEMPLATE, never guessed from the generated value, and anything not
known (attachments, activities, assets, invitations, stickies) is left to the generator.
"""

import json
import os
import re
import uuid

import schemathesis

_MANIFEST = os.environ.get("ORACLE_MANIFEST", "oracle/corpus/seed-manifest.json")
M = json.load(open(_MANIFEST))

# Same namespace as oracle/scripts/seed.py, so comment/link ids are reproducible without being
# listed in the manifest.
_NS = uuid.UUID("5a0a0a0a-0000-4000-8000-0123456789ab")
_uid = lambda name: str(uuid.uuid5(_NS, name))  # noqa: E731

ALPHA = M["projects"]["alpha"]["id"]
I3 = M["issues"]["i3"]

BY_NAME = {
    "slug": M["workspace"]["slug"],
    "project_id": ALPHA,
    "cycle_id": M["cycle"],
    "module_id": M["module"],
    "issue_id": I3["id"],
    "state_id": M["states"]["alpha:Todo"],
    "project_identifier": M["projects"]["alpha"]["identifier"],
    "issue_identifier": str(I3["sequence_id"]),
}

# `{pk}` means a different resource per route; decide from the segment that precedes it.
PK_BY_SEGMENT = {
    "projects": ALPHA,
    "cycles": M["cycle"],
    "modules": M["module"],
    "labels": M["labels"]["alpha:bug"],
    "states": M["states"]["alpha:Todo"],
    "issues": I3["id"],
    "work-items": I3["id"],
    "comments": _uid("comment:i3:1"),
    "links": _uid("link:i3:1"),
    "activities": _uid("activity:i3:1"),
    # ProjectMember ROW id, not the user id (the user id 404s: measured)
    "members": _uid("pmember:alpha:member"),
    "project-members": _uid("pmember:alpha:member"),
}


def _pk_for(path_template):
    m = re.search(r"/([a-z-]+)/\{pk\}", path_template)
    return PK_BY_SEGMENT.get(m.group(1)) if m else None


@schemathesis.hook
def map_path_parameters(context, path_parameters):
    """Replace generated path parameters with seeded ids where the route tells us which."""
    if not path_parameters:
        # Operations with no path parameters (users/me, user-assets, ...) pass None. Calling
        # dict(None) here crashed schemathesis's generator and silently untested those operations.
        return path_parameters
    out = dict(path_parameters)
    template = context.operation.path
    for name in list(out):
        if name == "pk":
            val = _pk_for(template)
        else:
            val = BY_NAME.get(name)
        if val is not None:
            out[name] = val
    return out
