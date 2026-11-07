"""Seed SYNTHETIC data into the pinned Plane for the v1 oracle.

Run INSIDE the api container through Django's shell (never via the HTTP API), e.g.

    docker compose -f oracle/compose.oracle.yml --env-file oracle/.env.oracle exec -T api \
        python manage.py shell -c "exec(open('/oracle/seed.py').read())"

Everything here is invented: example.test emails, fake tokens, lorem-free item titles. Ids are
uuid5 over a fixed namespace, so a rebuilt stack seeds byte-identical ids and recordings made
from it are reproducible (timestamps and server-generated ids are still normalised; see
oracle/NORMALISATION.md).

The script is idempotent on its deterministic ids and prints a JSON manifest between the
ORACLE_MANIFEST_BEGIN / ORACLE_MANIFEST_END markers for the harness to read.
"""

import json
import uuid
from datetime import timedelta

from django.utils import timezone

from plane.db.models import (
    APIToken,
    Cycle,
    CycleIssue,
    Issue,
    IssueActivity,
    IssueAssignee,
    IssueComment,
    IssueLabel,
    IssueLink,
    Label,
    Module,
    ModuleIssue,
    Project,
    ProjectIdentifier,
    ProjectMember,
    State,
    User,
    Workspace,
    WorkspaceMember,
)
from plane.db.models.state import DEFAULT_STATES

NS = uuid.UUID("5a0a0a0a-0000-4000-8000-0123456789ab")


def uid(name):
    return uuid.uuid5(NS, name)


def get_or_create(model, pk_name, defaults):
    """Create `model` with the deterministic id `uid(pk_name)` unless it already exists."""
    pk = uid(pk_name)
    obj = model._base_manager.filter(pk=pk).first()
    if obj is None:
        obj = model.objects.create(pk=pk, **defaults)
    return obj


# --- users ----------------------------------------------------------------------------------
USERS = {
    # key: (email, first, last, workspace role, project-Alpha role)
    "owner": ("oracle-owner@example.test", "Olive", "Owner", 20, 20),
    "member": ("oracle-member@example.test", "Milo", "Member", 15, 15),
    "guest": ("oracle-guest@example.test", "Gwen", "Guest", 5, 5),
    "outsider": ("oracle-outsider@example.test", "Otto", "Outsider", None, None),
}
users = {}
for key, (email, first, last, _, _) in USERS.items():
    user = User.objects.filter(email=email).first()
    if user is None:
        user = User(
            id=uid(f"user:{key}"),
            email=email,
            username=f"oracle_{key}",
            first_name=first,
            last_name=last,
            display_name=f"oracle_{key}",
            is_active=True,
            is_email_verified=True,
        )
        user.set_password("not-a-real-password-synthetic")
        user.save()
    users[key] = user

# --- workspace ------------------------------------------------------------------------------
workspace = get_or_create(
    Workspace,
    "workspace:oracle",
    dict(name="Oracle Workspace", slug="oracle-ws", owner=users["owner"], organization_size="1-10"),
)
for key, (_, _, _, ws_role, _) in USERS.items():
    if ws_role is not None:
        get_or_create(
            WorkspaceMember,
            f"wsmember:{key}",
            dict(workspace=workspace, member=users[key], role=ws_role),
        )

# --- projects -------------------------------------------------------------------------------
# Alpha is public to the workspace; Beta is secret and has only the owner, so a workspace
# member's API key gets a real "not allowed" answer from it.
projects = {}
for key, (name, ident, network, member_keys) in {
    "alpha": ("Oracle Alpha", "ORA", 2, ["owner", "member", "guest"]),
    "beta": ("Oracle Beta", "ORB", 0, ["owner"]),
}.items():
    project = get_or_create(
        Project,
        f"project:{key}",
        dict(
            name=name,
            identifier=ident,
            workspace=workspace,
            network=network,
            description=f"Synthetic project {name} for the v1 oracle.",
            project_lead=users["owner"],
        ),
    )
    ProjectIdentifier.objects.get_or_create(
        project=project, defaults=dict(name=ident, workspace=workspace)
    )
    for mk in member_keys:
        role = USERS[mk][4] if key == "alpha" else 20
        get_or_create(
            ProjectMember,
            f"pmember:{key}:{mk}",
            dict(project=project, workspace=workspace, member=users[mk], role=role),
        )
    projects[key] = project

# --- states, labels, work items -------------------------------------------------------------
states = {}
labels = {}
issues = {}
for pkey, project in projects.items():
    for st in DEFAULT_STATES:
        state = get_or_create(
            State,
            f"state:{pkey}:{st['name']}",
            dict(
                project=project,
                workspace=workspace,
                name=st["name"],
                color=st["color"],
                sequence=st["sequence"],
                group=st["group"],
                default=st.get("default", False),
                is_triage=st["group"] == "triage",
            ),
        )
        states[f"{pkey}:{st['name']}"] = state
    project.default_state = states[f"{pkey}:Backlog"]
    project.save(update_fields=["default_state"])

    for lname, color in (("bug", "#E5484D"), ("feature", "#3E63DD"), ("docs", "#30A46C")):
        labels[f"{pkey}:{lname}"] = get_or_create(
            Label,
            f"label:{pkey}:{lname}",
            dict(project=project, workspace=workspace, name=lname, color=color),
        )

today = timezone.now().date()
ALPHA_ITEMS = [
    # key, name, state, priority, assignees, labels, parent key, start offset, target offset
    ("i1", "Oracle item one", "Backlog", "none", [], [], None, None, None),
    ("i2", "Oracle item two", "Todo", "medium", ["member"], ["bug"], None, 0, 7),
    ("i3", "Oracle item three", "In Progress", "high", ["owner"], ["feature", "docs"], None, -2, 5),
    ("i4", "Oracle item four", "Done", "low", ["owner", "member"], [], None, -9, -1),
    ("i5", "Oracle item five", "Cancelled", "urgent", [], ["bug"], None, None, None),
    ("i6", "Oracle sub-item of three", "Todo", "none", ["member"], [], "i3", 1, 3),
    ("i7", "Oracle item seven", "Backlog", "low", [], ["docs"], None, None, None),
    ("i8", "Oracle item eight", "Todo", "medium", ["owner"], ["feature"], None, 3, 10),
    ("i9", "Oracle item nine", "In Progress", "none", [], [], None, None, None),
]
BETA_ITEMS = [
    ("b1", "Oracle beta item one", "Backlog", "none", [], [], None, None, None),
    ("b2", "Oracle beta item two", "Todo", "high", ["owner"], [], None, None, None),
]
for pkey, items in (("alpha", ALPHA_ITEMS), ("beta", BETA_ITEMS)):
    project = projects[pkey]
    for key, name, state, prio, assignees, lbls, parent, s_off, t_off in items:
        issue = Issue._base_manager.filter(pk=uid(f"issue:{key}")).first()
        if issue is None:
            issue = Issue.objects.create(
                pk=uid(f"issue:{key}"),
                project=project,
                workspace=workspace,
                name=name,
                state=states[f"{pkey}:{state}"],
                priority=prio,
                parent=issues.get(parent),
                description_html=f"<p>Synthetic description for {name}.</p>",
                description_stripped=f"Synthetic description for {name}.",
                start_date=today + timedelta(days=s_off) if s_off is not None else None,
                target_date=today + timedelta(days=t_off) if t_off is not None else None,
            )
            for a in assignees:
                IssueAssignee.objects.create(
                    issue=issue, assignee=users[a], project=project, workspace=workspace
                )
            for lb in lbls:
                IssueLabel.objects.create(
                    issue=issue, label=labels[f"{pkey}:{lb}"], project=project, workspace=workspace
                )
        issues[key] = issue

# comments and a link on item three, so the sub-resources have data
i3 = issues["i3"]
for n in (1, 2):
    if not IssueComment._base_manager.filter(pk=uid(f"comment:i3:{n}")).exists():
        IssueComment.objects.create(
            pk=uid(f"comment:i3:{n}"),
            issue=i3,
            project=projects["alpha"],
            workspace=workspace,
            actor=users["owner"],
            comment_html=f"<p>Synthetic comment {n}.</p>",
            comment_stripped=f"Synthetic comment {n}.",
        )
if not IssueLink._base_manager.filter(pk=uid("link:i3:1")).exists():
    IssueLink.objects.create(
        pk=uid("link:i3:1"),
        issue=i3,
        project=projects["alpha"],
        workspace=workspace,
        title="Synthetic link",
        url="https://example.test/oracle",
    )

# --- cycle and module -----------------------------------------------------------------------
now = timezone.now()
cycle = get_or_create(
    Cycle,
    "cycle:alpha:1",
    dict(
        project=projects["alpha"],
        workspace=workspace,
        name="Oracle Cycle One",
        description="Synthetic cycle.",
        start_date=now - timedelta(days=1),
        end_date=now + timedelta(days=13),
        owned_by=users["owner"],
    ),
)
module = get_or_create(
    Module,
    "module:alpha:1",
    dict(
        project=projects["alpha"],
        workspace=workspace,
        name="Oracle Module One",
        description="Synthetic module.",
        lead=users["owner"],
    ),
)

# Membership of the cycle/module and a couple of activity rows. Without these the detail routes
# cycle-issues/{issue_id}, module-issues/{issue_id} and activities/{pk} have nothing to return.
# (Activity rows are normally written asynchronously by a celery task after an API write; the ORM
# seed bypasses that, so they are created directly.)
for iss in ("i2", "i3"):
    get_or_create(
        CycleIssue,
        f"cycleissue:{iss}",
        dict(cycle=cycle, issue=issues[iss], project=projects["alpha"], workspace=workspace),
    )
get_or_create(
    ModuleIssue,
    "moduleissue:i3",
    dict(module=module, issue=issues["i3"], project=projects["alpha"], workspace=workspace),
)
for n, (verb, field, old, new) in enumerate(
    (("created", None, None, None), ("updated", "priority", "none", "high")), start=1
):
    get_or_create(
        IssueActivity,
        f"activity:i3:{n}",
        dict(
            issue=issues["i3"],
            project=projects["alpha"],
            workspace=workspace,
            verb=verb,
            field=field,
            old_value=old,
            new_value=new,
            comment=f"synthetic activity {n}",
            actor=users["owner"],
        ),
    )

# --- API tokens -----------------------------------------------------------------------------
# Fake, obviously-synthetic values. The plane_api_ prefix is what Plane itself generates.
TOKENS = {
    "owner": ("plane_api_oracle_owner_00000000000000000001", "owner", {}),
    "member": ("plane_api_oracle_member_0000000000000000002", "member", {}),
    "guest": ("plane_api_oracle_guest_00000000000000000003", "guest", {}),
    "outsider": ("plane_api_oracle_outsider_000000000000004", "outsider", {}),
    # a second token for the owner, used only on the low-limit service to record 429s
    "ratelimit": ("plane_api_oracle_ratelimit_000000000000005", "owner", {}),
    # 401 fixtures: valid format, but expired / deactivated
    "expired": (
        "plane_api_oracle_expired_00000000000000000006",
        "owner",
        {"expired_at": now - timedelta(days=1)},
    ),
    "inactive": (
        "plane_api_oracle_inactive_0000000000000000007",
        "owner",
        {"is_active": False},
    ),
}
tokens = {}
for key, (value, user_key, extra) in TOKENS.items():
    tok = APIToken.objects.filter(token=value).first()
    if tok is None:
        tok = APIToken.objects.create(
            user=users[user_key],
            token=value,
            label=f"oracle-{key}",
            description="Synthetic oracle token",
            workspace=workspace,
            **extra,
        )
    tokens[key] = tok.token

manifest = {
    "workspace": {"id": str(workspace.id), "slug": workspace.slug},
    "users": {k: {"id": str(u.id), "email": u.email} for k, u in users.items()},
    "projects": {k: {"id": str(p.id), "identifier": p.identifier} for k, p in projects.items()},
    "states": {k: str(s.id) for k, s in states.items()},
    "labels": {k: str(l.id) for k, l in labels.items()},
    "issues": {k: {"id": str(i.id), "sequence_id": i.sequence_id} for k, i in issues.items()},
    "cycle": str(cycle.id),
    "module": str(module.id),
    # members/{pk} and project-members/{pk} take the ProjectMember ROW id, not the user id
    "project_members": {
        f"{pk}:{mk}": str(uid(f"pmember:{pk}:{mk}"))
        for pk, mks in (("alpha", ("owner", "member", "guest")), ("beta", ("owner",)))
        for mk in mks
    },
    "comments": {"i3:1": str(uid("comment:i3:1")), "i3:2": str(uid("comment:i3:2"))},
    "links": {"i3:1": str(uid("link:i3:1"))},
    "activities": {"i3:1": str(uid("activity:i3:1")), "i3:2": str(uid("activity:i3:2"))},
    "tokens": tokens,
}
print("ORACLE_MANIFEST_BEGIN")
print(json.dumps(manifest, indent=2, sort_keys=True))
print("ORACLE_MANIFEST_END")
