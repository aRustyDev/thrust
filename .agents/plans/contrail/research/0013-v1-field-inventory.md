# Research 0013: the `/api/v1` field inventory for issue #58

What the 32 in-scope v1 operations return and accept, set against the schema as migrations 0001 to 0013 leave it.
This is the input to migration `0015` and to the serializers; it is step 1 of the #58 plan.

Sources, all read without running Plane: the pinned schema `tools/oracle-v1/schema/openapi.v1.json`; the 105 corpus
exchanges that fall on the 32 operations (`tools/oracle-v1/corpus/`); the pinned Plane source at tag
`oracle/v1-pin-20261018` (`apps/api/plane/api/views/state.py`, `db/models/state.py`, `app/permissions/project.py`).
Anything not read from one of those is marked **[unverified]**.

## 1. The operations

The schema has exactly 32 operations under the tags Projects 8, States 5, Labels 5 and Work Items 14, as issue #58
says. The corpus exercises 28 of them: no exchange exists for `PATCH`/`DELETE` on the legacy `.../issues/{pk}/`
path, the workspace-level `issues/{KEY}-{n}/` lookup and `issues/search/`. Statuses seen per operation are in the
recorder's output; the ones that matter for the code are in section 3.

## 2. Column deltas for migration 0015

| Resource | v1 fields with no column today | Decision |
|---|---|---|
| Project | `deleted_at` | add; the `(workspace_id, identifier)` and `(workspace_id, name)` uniques become partial `WHERE deleted_at IS NULL`, as Plane's are |
| Project | `total_members`, `total_cycles`, `total_modules`, `is_member`, `member_role`, `sort_order`, `is_deployed`, `cover_image_url` | derived, no column. `member_role` is the caller's `project_members.role` as 5/15/20, `sort_order` the caller's `project_members.sort_order`, `is_deployed` false |
| Project | alpha has `total_cycles: 1`, `total_modules: 1` | Thrust has no cycles or modules: corpus exception plus a DEVIATIONS entry, not a column |
| State | `is_triage`, `external_source`, `external_id`, `updated_by`, `deleted_at` | add the three integration/audit columns and `deleted_at`. **Correction (built):** `is_triage` is not a column here: it is whether the group is `triage` (a client's `is_triage` is read and dropped; D-031). `slug` is derived (`slugify(name)`, recomputed on every save in Plane). `UNIQUE (project_id, name)` becomes partial on `deleted_at IS NULL` |
| Label | `external_source`, `external_id`, `updated_by`, `deleted_at` | add; same partial unique |
| Work item | `parent_id` (self reference), `point` (integer) | add. D-028 refused a parent; v1 accepts one (same project, not itself) and the fixture needs i6 under i3 |
| Work item | `type`/`type_id`, `estimate_point`, `description_binary` | no column. Thrust has no issue types or estimates, so reads return `null` and a non-null write gets Plane's own 400 for a missing object (`Invalid pk "<id>" - object does not exist.`). Unobservable by the oracle, recorded as a deviation |
| Work item | `created_by` is `NOT NULL` | **open, O-1**: seeded rows have `created_by: null` in every corpus body (`seed.py` runs without a request user), and API creates carry the creator |

O-1: either make `work_items.created_by` nullable (Plane's model allows it; `WorkItemRow.created_by` becomes an
`Option`, which touches compat's `issue_json`) or keep it and register a seeded-row exception. Recommended: nullable,
because the exception would blind the comparison on exactly the field a regression is likely to break.

The `sort_order` of a work item is a `bigint` here and a float in v1 (`75535.0`): serialize as a float.
`archived_at` is a `timestamptz` here and a `date` in the v1 `Issue`.

## 3. Behaviour the inventory found (corrections to the #58 plan are marked)

- **State create sequence (correction).** The plan said `last + 10000`. Plane's `State.save()` sets
  `sequence = max(sequence of the project's non-triage, non-deleted states) + 15000` whenever the project has one,
  **ignoring any `sequence` the client sent**. That is why the seeded states read 15000, 30000, 45000, 60000, 75000
  and the created one 90000.0, although `DEFAULT_STATES` says 15000, 25000, 35000, 45000, 55000. The fixture must
  carry the observed values.
- **Triage is hidden.** `StateManager` excludes the `triage` group, and the v1 state list and detail filter
  `is_triage=False`. The corpus list returns 5 of the 6 seeded states; the project summary counts 5.
- **State rules.** Create returns **200**. A duplicate name gives 409 `{"error":"State with the same name already
  exists in the project","id":"<id>"}`; a duplicate `(external_source, external_id)` gives 409 `{"error":"State with
  the same external id and external source already exists",...}`. Delete: the default state gives 400
  `{"error":"Default state cannot be deleted"}`; a state that holds work items gives 400
  `{"error":"The state is not empty, only empty states can be deleted"}`; success is 204. The "holds work items"
  check is `Issue.objects.filter(state=..)`, and `Issue.objects` is a manager that **excludes** archived items, drafts,
  items in a triage state and items of an archived project. So a state whose only items are archived or drafts is
  deletable in Plane, which leaves those items pointing at a deleted state (reproduce it, record it as a quirk).
- **Permissions (correction, read from the pin's `state.py` while building the states).** The v1 state views use
  `ProjectAdminPermission` for **every write** (create, change, delete): an active **project** membership with role
  **Admin**. A project Member is refused, as is a workspace admin who is not in the project. A read needs an active
  project membership. A refused write is 403 `{"detail":"You do not have permission to perform this action."}` with
  no rate headers. The earlier text here (a write by Admin or Member) was wrong for states. Labels use
  `ProjectMemberPermission`: a read needs an active project membership, and **create, change and delete need a project
  role of Admin or Member** (the comment on its last branch says "only Project Admins", but the code it sits on lists
  `ROLE.ADMIN` and `ROLE.MEMBER`; resolved by reading `app/permissions/project.py` in the labels stage). A guest, a
  stranger and a workspace admin who is not in the project are refused.
- **Label rules** (the views are in `api/views/issue.py`, not in a file of their own).
  - Create is 201; a duplicate name 409 `{"error":"Label with the same name already exists in the project","id":"<id>"}`;
    renaming onto an existing name 400 `{"error":"The payload is not valid"}`; a repeated delete 404
    `{"error":"The requested resource does not exist."}`.
  - **One queryset serves a read, a change and a deletion**: live labels of the project, the caller an active member, the
    project **not archived**. A change or a deletion of a label of an archived project is a 404 (a state's is not).
  - A creation looks the clashing label up by the name **as sent**, which DRF trimmed: a name with spaces round it that
    clashes finds no row, and the view raises (500). The same is true of the state view.
  - The list has no `order_by` of its own (the view orders by a URL keyword that no route has): `-created_at`. The detail
    read passes neither `fields` nor `expand` to its serializer.
  - A new label's `sort_order` is the greatest live one in the project plus **10000** (a state's is 15000), the client's value
    ignored; the first keeps its own or 65535.
  - `parent` is a `PrimaryKeyRelatedField` over every label: a value Django cannot read as a UUID raises a Django
    `ValidationError`, and **the serializer catches it per field** and files `“x” is not a valid UUID.` under `parent`, with the
    other fields' errors beside it. **Correction (2026-11-10, measured on a running pin by the conformance gate's stage):** this
    note first read the source as saying the error escapes the field and the base view answers `{"error":"Please provide valid
    detail"}`; it does not. That bare 400 is for a Django `ValidationError` raised by the VIEW (a query parameter that names a row
    by a key Django cannot read, e.g. the search's `project_id`), not by a body's field (D-032 keeps the answers, and narrows the
    field to the project's own labels).
  - `label.delete()` marks the row and hands the labels under it (the `parent` foreign key cascades) and the work items'
    join rows to a background task; Thrust does all of it in one statement.
  - The body of every v1 view is read before the handler runs (`BaseAPIView.initial` reads `request.data`), so a body that
    cannot be parsed is refused before any lookup; this corrects the first stage's states, which looked first.
- **Work-item responses carry 29 keys, not the schema's 27**: `assignees` and `labels` (uuid lists) are in every
  body and absent from the `Issue` schema.
- **`expand`**: `state` becomes `{color, group, id, name}`; `assignees` becomes user briefs `{avatar, avatar_url,
  display_name, email, first_name, id, last_name}`; `labels` become full label objects. **`fields`** projects the
  body to the named keys (EX-007). **`order_by`** takes a `-` prefix (`sequence_id`, `-priority`, `name`).
  The corpus uses each on work items; projects use `fields`, `order_by`, `per_page` and `cursor`.
- **Cursor.** `per_page:page:is_prev`: `next_cursor` is `1000:1:0` and `prev_cursor` `1000:-1:1` on a one-page
  list. The envelope has 12 keys; `per_page` above 1000 is 400, `0` and `-1` are 500 (EX-001/002).
- **Creates record the actor** in `created_by` and `updated_by` stays `null` until an update.
  A work item created with no `state` lands in the project's default state (Backlog).
- **Sequence ids and sort orders** continue from the project counter and are never reused after a delete
  (first created item is `sequence_id` 10, `sort_order` 85535.0, which is the previous maximum plus 10000).

## 4. Not read, to settle against the corpus while building each resource

Label parent handling and `sort_order` defaults; project archive and unarchive effects on lists (`include_archived`
on `projects-lite/`); work-item `PATCH` rules and the legacy `issues/` family's differences; the 405 and 415 bodies on
in-scope paths; whether a workspace Admin who is not a project member can read a project (Thrust's `authorize`
joins `project_members` with a left join, Plane's permission does not); the user-profile fields behind the assignee
brief (`user_profiles`, 0008, may not carry `display_name` or `avatar`).
