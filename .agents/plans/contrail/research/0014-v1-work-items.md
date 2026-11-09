# 0014: the v1 work items, read from the pin before building them (#58)

Source: the pin `oracle/v1-pin-20261018` (`apps/api/plane/api/views/issue.py`, `api/serializers/issue.py`, `db/models/issue.py`,
`api/views/base.py`, `utils/paginator.py`, `utils/order_queryset.py`) and the corpus `tools/oracle-v1/corpus/work_items.json`. The
states and labels notes are in 0013. This note is the second stacked PR's design: what the pin does, what is built on what exists, and
what is a departure (D-033, confirmed by the owner 2026-11-09).

## 1. The 14 operations

Two path families with the same views (`/issues/` and `/work-items/`): list, create, retrieve, change (PATCH), delete under
`projects/{project_id}/`, and under the workspace the search (`.../search/`) and the lookup by `{KEY}-{n}`. The comments, links,
activities, relations and attachments routes are other tags; the corpus records them, this PR does not build them. `PUT` upsert
(`IssueDetailAPIEndpoint.put`) is in no schema operation of these tags, and is not built.

## 2. What the pin does

**Permissions and querysets.** `ProjectEntityPermission`: a read needs an active **project** membership (a guest too, and a guest
sees every work item: the v1 queryset has no creator filter, which the compat plane has); a write (create, change, delete) needs a
project role of Admin or Member. The list, the detail and the identifier lookup read `Issue.issue_objects`: not deleted, not archived,
not a draft, not in a triage state, project not archived. A change and a delete read `Issue.objects` (live, **archived and draft
included**).

**Delete** is further gated in the handler: the creator or a project Admin, else 403 `{"error": "Only admin or creator can delete the
work item"}` (counted, with rate headers, unlike a permission refusal). It marks the row and hands the rest (sub-issues, join rows,
comments) to a background task.

**Lookup by identifier.** The route is `<project_identifier>-<issue_identifier>`, so a `-` in it splits at the **last** hyphen. The
permission looks for a membership of a project with that identifier (none: 403 `detail`, uncounted); a non-decimal number is 404
`{"error": "Work item not found"}`, a decimal one that names nothing 404 `{"error": "The requested resource does not exist."}`.

**Search.** Any authenticated key may call it: the queryset is the caller's own projects (active member, project not archived), so a
stranger gets `{"issues": []}`, not a 403. `search` empty or missing: `{"issues": []}`. A match is `name` or the project's identifier
containing the text (case-insensitive), or the `sequence_id` equal to a whole number in the text (`\b\d+\b`); `workspace_search` not
`"false"` ignores `project_id`; `limit` defaults to 10 and is `int()`ed (text: 500; negative: 500). A row is `{name, id, sequence_id,
project__identifier, project_id, workspace__slug}`, newest first.

**List.** `pql` or `filters` (any value): 400 with a fixed body. `external_id` and `external_source` both given: **one object** (not a
page), 404 if none. `order_by` goes through an allowlist (`created_at updated_at sequence_id sort_order target_date start_date
completed_at archived_at priority state__name state__group assignees__first_name labels__name issue_module__module__name`, with an
optional `-`; anything else is the default `-created_at`). The total is `Issue.issue_objects` of the project. `fields` projects the
body; `expand` takes `state` (`{color, group, id, name}`), `assignees` (user briefs), `labels` (full label objects), and the base
serializer's others (`project`, `workspace`, `created_by`, `updated_by`, `parent`).

**The body** has 29 keys (corpus): `archived_at assignees completed_at created_at created_by deleted_at description_binary
description_html estimate_point external_id external_source id is_draft labels name parent point priority project sequence_id
sort_order start_date state target_date type type_id updated_at updated_by workspace`. `assignees` and `labels` are id lists.

**Create.** The serializer's fields are checked in order and what is wrong is one 400; then `validate()` raises at its first
refusal as a non-field error: start after target ("Start date cannot exceed target date"), HTML that does not parse or is not clean,
**assignees that are not active project members of role Member or Admin** ("Assignees [...] are not active members of this
project": refused, where the compat plane drops them), labels not of the project ("Labels [...] do not belong to this project"), a
state not of the project ("State is not valid please pass a valid state_id"), a parent not of the project ("Parent is not valid
issue_id please pass a valid issue_id"), an estimate point that is not (always, here). A relation that names no row at all is the
field's `Invalid pk "<value>" - object does not exist.` (the state's queryset leaves out the triage state). `external_id` and
`external_source` both named and held by another item: 409 `{"error": "Issue with the same external id and external source already
exists", "id": "<id>"}`. The item is numbered from the project's counter, takes the project's default state (and `completed_at` if
that is a completed one), is placed 10000 after the greatest `sort_order` in its state, and takes the project's default assignee
when none is named and that person may be assigned. Then the view **overwrites** `created_at` and `created_by` with the body's raw
values (default now, the caller) and saves only those; the response was built before, so it does not show them.

**Change.** The item is found (`Issue.objects`, 404), then the serializer (partial) reads the body; the 409 for a key follows the
states' rule (`external_id` named, different from the item's own, and another item has it with the named source or else the item's
own: the answer names the item being changed). Assignees and labels named are replaced whole. `updated_at` and `updated_by` are
set; `completed_at` follows a change of state.

**Writable fields the serializer does not protect**: `created_at`, `created_by`, `deleted_at`, `archived_at`, `is_draft`,
`sort_order`, `sequence_id`, `type_id`, `estimate_point` (it excludes only `description_json` and `description_stripped`, and makes
`id workspace project updated_by updated_at completed_at` read-only).

## 3. Built on what exists

The compat plane's `create_issue`, `update_issue`, `work_item_page` and `soft_delete_work_item` already do the numbering, the
default state, `completed_at`, the history and the outbox events. v1 reuses them. What the surfaces differ in is carried by the
caller's proof, `IssueAccess`, not by a second implementation: a v1 access sees every item whatever the caller's role, accepts a
parent, and is made by a v1 permission (a guest may not change an item they made; the compat plane lets them).

## 4. Decisions

1. **Schema (migration 0015).** `work_items.created_by` becomes nullable (decided on #58's thread: the seeded items have none);
   `parent_id` (self reference, same project, checked by the use-case) and `point` (0 to 12, a check) are added. `WorkItem.created_by`
   is an `Option`, which touches the compat plane's reads.
2. **Stored.** name, description, priority, state, parent, point, the two dates, assignees, labels and the external key. **Read and
   checked as the serializer does, and not stored** (D-033, confirmed): `created_at`, `created_by` (so a key cannot forge who made an
   item), `deleted_at`, `archived_at`, `is_draft`, `sequence_id`, `sort_order` on a creation, `type_id`, `estimate_point` (no such
   rows here: any value but null is the `Invalid pk` of a row that is not there).
3. **Parents.** A work item may have a parent that is a work item of the same project (the pin allows archived and draft ones, and
   itself). The compat plane keeps refusing one (D-028 stays for its writes; its reads now show a parent that v1 set).
4. **Order by `labels__name`, `assignees__first_name` and `issue_module__module__name`** are not offered by the compat adapters; a
   request for one is ordered by `-created_at` (D-033, confirmed) unless building them proves cheap.
5. **Sub-issues on delete.** The pin's background task deletes the sub-issues with their parent; here the items under a deleted
   item are deleted with it in the same transaction, as the labels' cascade is.
6. **`expand`** is built for `state`, `assignees` and `labels` (the corpus has them); the base serializer's other expansions are
   ignored (D-033, confirmed), as for states and labels (D-032).

## 5. Stages (one commit each, tests with each)

1. Schema and ports: `created_by` nullable, `parent_id`, `point`; both adapters; the compat plane's reads follow.
2. The reads: a v1 access, a page of items for any member, the lookup by identifier, the workspace search.
3. The writes: create, change and delete by the pin's rules and in its words; sub-issue cascade.
4. The routes and wire shape: both families, DRF's fields, `fields`, `expand`, the list's single-object mode and its refusals.
5. D-033, the PR body, the corpus-shaped checks.
