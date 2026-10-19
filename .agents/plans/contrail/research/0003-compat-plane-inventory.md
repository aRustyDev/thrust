# 0003 — Compat-plane inventory and sizing (spike 0009, desk half)

> **desk half. Two runs of the same scripts:**
> - **pre-pin, source commit `300c2009d4fdcf5369b92e108da0c2de4212eb4a`** (`v1.4.3-7-g300c2009d4`, branch `docs/plane-on-cloudflare-spec`,
>   apps/ and packages/ clean): sections 1-14, output in `scripts/compat-inventory/out/`.
> - **pinned, source commit `ba85d4d3d332b5380bebca4b9e0143b1127de907`** (tag `oracle/v1-pin-20261018`, upstream preview `879ce8b5c6`; the fork
>   merged with 220 upstream commits; bead `plane-b6a`): section 15, output in `scripts/compat-inventory/out-pinned/`. The scanned worktree's
>   HEAD is `72b2e01497` (the pin plus oracle-harness commits); its `apps/api` tree is proven identical to the pin (section 15.1).
>
> Sections 1-14 are the **pre-pin** numbers and are left as first written (one correction: four slice trigger line numbers, section 15.1).
> **Section 15 holds the pinned numbers, every delta, and what moved in the estimates; the pinned run is the one that describes the oracle.**
> Everything comes from source; no system was run, no HAR exists, no call frequency is known.
>
> Gates ADR-0034, F-X13. Inputs to Q-0010.1, Q-0011.1, Q-0009.1, Q-R2 (section 12). Nothing outside `scripts/compat-inventory/`
> and this file was written; the Plane trees were only read.
>
> **Corrections (round 27, from `research/0007` "Corrections" 6 to 10):** the pinned React client appends `/` to every API URL (`normalizeAPIRequestURL` in `apps/web/services/api.service.ts`), so the slashless-call concern in sections 5 and 9 row 4 is moot for the stock UI; at the pin the 401 interceptor skips `/` and URL-encodes the path, and `?t=` survives only on `users/me/settings/` (sections 7.5 and 9); the pinned tree has 55 `self.paginate(` sites and 28 `order_by` sanitiser call sites. These do not change the slice or the estimates.
>
> Evidence tags: **[computed]** = reproduced by the scripts; **[inferred]** = my judgement from computed facts (unit costs, liveness,
> defaults); **[unverified]** = could not be checked from source. Untagged numbers are [computed]. In section 15, **[diff]** = found by reading
> a plain `diff` of two source files by hand (not produced by the scripts, so it will not reappear on the next re-run unless someone repeats it).
> Every table below is regenerated into `scripts/compat-inventory/out/tables.md` (pinned: `out-pinned/tables.md`) by `run_all.py`; counts quoted in prose come from
> `server-summary.json` (`_facts`, `_reconciliation`), `client-scan-stats.json`, `join-report.json`, `effort-summary.json` (`facts`) and
> `shapes-anchors.json`, all in the same output directory.

## 1. Answer first

*Pre-pin numbers (commit `300c2009d4`). The pinned values and what moved are in section 15.1-15.6; no conclusion below is invalidated (section 15.11).*

1. **Route counts match the review exactly where the review was exact, and the one gap is explained.** `plane.app` = **233** `path()`
   entries (441 method x path rows). Auth + public/space = **41 + 25 = 66**. `/api/v1` = **68** reachable entries (132 rows) vs the review's
   "about 70" (raw `path(` lines = 70; 3 `estimate.py` + 3 `schema.py` lines are never included; 2 `include(router.urls)` lines expand to
   6 entries). The review's 233/66/70 **omit** `plane.license` (**14** entries, 21 rows: instance info + god-mode), which is on the boot
   path of every app, plus health/robots (2) and the conditional drf-spectacular UI (3). Grand total **386** entries.
2. **Join.** 502 axios-style call sites (71 `APIService` subclasses) + 32 non-axios path references (form posts, redirects, fetch, asset
   URLs). Of the 529 call sites that target the Plane API, **478 match a server route + verb, 51 do not** (29 are dead service methods,
   22 look live and point at routes this server does not have: EE dashboards/integrations/Slack/GitHub, `bulk-operation-issues`,
   `PATCH sidebar-preferences/{key}/` ...). **204 of 545 server rows (internal + space + instance + auth) are never used by this React
   tree** (internal 171 of 441 = 39%, including all 13 `PUT` rows and every `intake-issues`/`intakes`/`inboxes` alias).
3. **Tiering** (FEATURES.md tiers; 545 rows): P0 **291**, P1 29, P2 8, admin-only 13 (apps/admin is slated to retire), not-used 204.
4. **Minimum P0 slice** (stock React UI: sign up, onboard, create workspace, create project, create work item, see it in the list):
   **48 route rows / 42 distinct paths** (+3 to open the item), of which 46 are P0-tier and 2 are P1-tier features that sit on the P0 path
   (`project-stats` F-D07, `users/me/workspaces/invitations` F-A07). Effort **38.6 person-days likely (26-66)**, 12.5 of which is shared
   machinery. Full P0 compat (291 rows) **207 pd likely (138-364)**; every row of every compat surface 328 pd (219-575). Unit costs are
   judgement [inferred]; class counts are computed.
5. **Sizing surprise:** creating a project through the *unmodified* UI uploads a random default cover image through the **presigned
   asset flow** (4 asset routes incl. the 302 image read, plus a browser POST to the object store), and the project page fires **13 reads on every visit** (the
   workspace wrapper fires 9 more on every workspace page). Even an empty project needs ~46 shape-correct endpoints.
6. **Top 5 quirks that bite** are in section 10; **React behaviours cheaper to fix in the Q-R2 areas** are in section 9. The biggest
   single lever is the **auth client** (13 files, 21 reference sites): replacing the CSRF-token + HTML-form-POST + `302 ?error_code=`
   flow with JSON removes about 16 pd of emulation [inferred] and 41 of the 545 rows.
7. **Scan accuracy:** `this.<verb>(` call-site recall **502/502** (independent regex); independent literal-path recall **261/262 = 99.6%**
   (the one miss is the regex instrument's own limit on a nested template, the scanner extracted it); hand-checked precision **30/30**
   call sites on verb, path, query, body, response type and catch shape (section 4).
8. **Not done here** (waits on spike 0008): HAR, call frequency, response bodies, the replay harness. The dynamic half's checklist is
   section 11.

## 2. Reproduce, and re-run after the pin

Tooling is stdlib-only Python through `uv run` (about 14 s for the whole run). Layout: `scripts/compat-inventory/`.

```sh
cd .agents/plans/contrail/scripts/compat-inventory
uv run run_all.py --root <plane tree root> --out out        # everything below; exit 2 if the P0 slice or tag table is stale
uv run server_routes.py --root <root> --out <dir>            # step 1 alone (also client_calls.py, join_tag.py, shapes_probe.py, ...)
# flags: --vcs files (default: stamp the commit by reading <root>/.git, never runs git) | git (rev-parse/describe/status)
#        --verdicts <csv> (hand-check verdicts for THIS tree)   --label <text> (recorded as tree_label, e.g. the pin tag)
```

| script | does | output (in `out/`) |
|---|---|---|
| `server_routes.py` (+`pyindex.py`) | walks `plane/urls.py` with `ast`: includes, router registrations, `as_view({...})` maps, `http_method_names`, converters | `server-routes.csv + .json` (one row per method x path, handler `file:line`, serializer names, AST facts), `server-route-entries.json`, `server-summary.json` (counts + reconciliation) |
| `client_calls.py` (+`ts_lex.py`, `client_overrides.py`) | TS lexer + class/member parser: every `this.<verb>(` in `APIService` subclasses, plus literal `/api/`, `/auth/` references (forms, redirects, fetch, asset URLs) | `client-calls.csv + .json`, `client-refs.csv + .json`, `client-scan-stats.json` |
| `join_tag.py` (+`feature_rules.py`, `effort_model.py`) | segment-wise path join, FEATURES.md tier parse, tags, dispositions, effort class | `endpoint-inventory.csv + .json` (**the committed inventory**), `join-matches.csv`, `join-report.json`, `join-unmatched.json` |
| `shapes_probe.py` | shape-catalogue facts with `file:line` | `shapes-anchors.json` |
| `p0_slice.py` + `effort_report.py` | hand-traced minimum slice, **validated against the join** (a stale entry fails the run) | `effort-summary.json` |
| `precision_recall.py` | independent recall regex, seeded hand-check sample (`--verdicts` picks the verdicts file; a verdict whose cited file moved marks the check `stale`) | `precision-recall.json`, `handcheck-sample.csv` |
| `layout.py` (used by all of the above) | resolves every hard-coded path through old/new locations (`apps/web/core/x` and `apps/web/x`, `packages/editor/src/core/x`, ...), **fails loudly naming the path and every variant tried**, records what moved, what is optional and absent, which shape anchors were violated, which client dirs are not scanned and whether they contain `/api`/`/auth` literals, and whether each slice trigger line moved | `layout-report.json` |
| `compare_runs.py`, `render_tables.py` | pre/post-pin diff (contract fields, serializer shapes, **handler source text**, client calls, join, effort, shapes, slice); markdown tables (slice triggers shown as resolved in the scanned tree) | stdout (+ `--json`), `tables.md` |
| `verify_api_tree.py` | recomputes the git tree sha of `<root>/apps/api` from files (no git) and compares it with `oracle/PIN` `apps_api_tree` | stdout (+ `--out`), `pin-verify.json` |

Determinism: two consecutive runs produce byte-identical `out/` (`diff -r` clean). Hand-curated inputs (everything that is judgement) are
confined to four small files: `feature_rules.py` (route -> feature ids), `client_overrides.py` (10 hand-resolved client paths, each with
its reason), `p0_slice.py` (the slice, each entry with trigger `file:line`), `effort_model.py` (unit costs).

**Re-run after the pin, and after the next upstream sync** (this is exactly what produced `out-pinned/`; do it in this order):

```sh
cd .agents/plans/contrail/scripts/compat-inventory
# 1. the previous result is the baseline: out/ (pre-pin, commit 300c2009d4) and out-pinned/ (pin ba85d4d3d3) are both kept in the repo
# 2. run against the new tree. --vcs files (default) never runs git against it; the trailing flags are optional
uv run run_all.py --root <new tree> --out out-<label> --label '<pin tag -> sha>' --verdicts handcheck-verdicts-<label>.csv > run.log 2>&1
uv run verify_api_tree.py --root <new tree> --out out-<label>/pin-verify.json     # only when the tree has oracle/PIN
# 3. what changed (needs both scanned trees on disk for the handler-body section; else pass --old-root/--new-root)
uv run compare_runs.py --old out-pinned --new out-<label> --json out-<label>/delta.json
# 4. then, by hand:
#    - exit 2 or non-empty `slice_validation_errors` in RUN.json: fix p0_slice.py (the error names the stale entry and its trigger file:line)
#    - layout-report.json: `paths_moved_*` (fine, informational), `optional_paths_absent`, `anchor_checks_violated` (a shape anchor drifted: read it),
#      `unlisted_dirs_not_scanned_as_clients` with `api_literal_hits > 0` (a new client dir: add it to client_calls.py), `slice_triggers.drift`
#    - a `LayoutError` from run_all.py names the missing path and every variant tried: add the new location to MOVE_RULES in layout.py
#    - `unmapped_server_routes` non-empty: add a rule to feature_rules.py
#    - `override_keys_unused` in client-scan-stats.json: re-read the call site, update client_overrides.py
#    - `precision_handcheck.stale: true` in precision-recall.json: the tree changed, re-do the 30-site hand check
#      (read handcheck-sample.csv, write handcheck-verdicts-<label>.csv, re-run with --verdicts)
#    - re-render this file's tables from tables.md; update the section 15 numbers
```

Resolved watch list (section 15.8): the woven OIDC addition (`authentication/provider/oauth/oidc.py`, 4 routes under `/auth/oidc*` and
`/auth/spaces/oidc*`) **survived the merge byte-identical**. The 3 `ENABLE_DRF_SPECTACULAR` routes are off by default.

## 3. Server route inventory (step 1)

Surfaces and their mount points come from `plane/urls.py` (ROOT_URLCONF), not from a directory listing.

| surface | path() entries | method x path rows | distinct paths | rows by method |
|---|---|---|---|---|
| internal (`/api/`, `plane.app`) | 233 | 441 | 233 | DELETE 78, GET 186, PATCH 60, POST 104, PUT 13 |
| space (`/api/public/`) | 25 | 42 | 25 | DELETE 7, GET 21, PATCH 4, POST 10 |
| auth (`/auth/`) | 41 | 41 | 41 | GET 21, POST 20 |
| v1 (`/api/v1/`) | 68 | 132 | 67 | DELETE 25, GET 56, PATCH 21, POST 28, PUT 2 |
| instance (`/api/instances/`, `plane.license`) | 14 | 21 | 14 | DELETE 3, GET 8, PATCH 2, POST 8 |
| web (`/`) | 2 | 2 | 2 | health, `robots.txt` |
| schema (`/api/schema/*`, only if `ENABLE_DRF_SPECTACULAR=1`) | 3 | 3 | 3 | GET |

**Reconciliation with REVIEW-NOTES (every difference explained):**

| conf | raw `path(` lines | reachable entries | note |
|---|---|---|---|
| `app/urls/*.py` | 233 | 233 | exact match with the review's 233 |
| `authentication/urls.py` + `space/urls/*.py` | 41 + 25 | 41 + 25 | exact match with the review's 66 (which it counted as "auth/public/space") |
| `api/urls/*.py` | 70 | 68 | `estimate.py` (3 lines) is **never included** by `api/urls/__init__.py`; `schema.py` (3) is never included either (spectacular routes are added conditionally in `plane/urls.py`); `invite.py` and `sticky.py` each hold one `include(router.urls)` line that expands into 3 entries (a DRF `DefaultRouter` API-root view + list + detail): 70 - 3 - 3 - 2 + 6 = 68. The two API-root entries are the same URL (`/api/v1/workspaces/{slug}/`). |
| `license/urls.py` | 14 | 14 | not counted by the review |
| `web/urls.py` | 0 | 2 | `urlpatterns = [path(...)]` written on one line |
| `plane/urls.py` | 9 | 3 | 6 are the `include()` mounts; 3 are the conditional schema routes |

Facts that matter for the facade:
- **How handlers are bound** (all 682 rows, every surface): 410 plain `APIView`-style classes (method = handler name), 256 `viewset` maps
  (`{"get": "list", ...}`), 14 DRF router rows (12 list/detail + 2 API-root views), 2 function views. 77 rows (73 on the compat surfaces) have no
  project handler at all (DRF default action over `get_queryset()`).
- **No regex routes, no `re_path`** except the debug-toolbar mount (skipped, `DEBUG` only). Converters in use: `str`, `uuid`, plus two
  mixed-segment patterns (`<project_identifier>-<issue_identifier>` in `work-items/` and v1, and `reset-password/<uidb64>/<token>/`).
- **Duplicate surfaces:** the same handler is reachable under two names in several places (`inbox-issues` / `intake-issues`,
  `inboxes` / `intakes`, v1 `issues` / `work-items`). The React tree uses only one of each pair (section 5).
- **Permission model** (545 compat rows): 235 use `@allow_permission` (roles 5/15/20 at project or workspace level, 26 of them also with
  `creator=True, model=...`), 310 rely on a class `permission_classes` (138 `IsAuthenticated`, 29 `AllowAny`, 15 `InstanceAdminPermission`,
  the rest workspace/project permission classes). Full breakdown: `tables.md` T9. This is the input table for the permission seam (F-E00b).
- **98 of the 545 compat rows dispatch a Celery task** (`.delay(` in the handler body; 73 of the 291 P0 rows): activity log, notifications, webhooks,
  versions, recent-visit tracking and telemetry are emitted from the API handlers, so a compat handler is not a pure CRUD passthrough: it must write to
  the outbox (ADR-0046).
- Routes added by the woven fork: `/auth/oidc/`, `/auth/oidc/callback/` and the `spaces/` variants (4), and `IS_OIDC_ENABLED` in the instance
  config (`license/api/views/instance.py`).

## 4. Client call-site inventory (step 2)

Scope scanned: `packages/services/src`, `apps/web`, `apps/space` (as asked) plus `apps/admin`, `apps/live` and the remaining `packages/*`
(labelled separately, because `apps/live` is a client of the internal API and `apps/admin` is a client of `/api/instances/`). 3,283 `.ts`/`.tsx`
files, excluding `node_modules`, build output, tests and stories.

| measure | value |
|---|---|
| `APIService` subclasses | 71 (two base classes: `apps/web/core/services/api.service.ts`, `packages/services/src/api.service.ts`) |
| `this.get/post/put/patch/delete/request(` call sites | **502** (web 341, pkg-services 154, live 7; `apps/space` and `apps/admin` have none: they use `@plane/services` and form posts) |
| verbs | GET 209, POST 140, DELETE 83, PATCH 66, **PUT 4** (2 are the presigned-upload helper, so 2 real API PUTs, both dead) |
| path resolution | 483 literal, 9 derived (ternary / enum fan-out), **10 hand-resolved** (`client_overrides.py`), 0 unresolved |
| non-axios path references | 32: `form-action` 9 + 2 dynamic, `redirect` (`window.location.assign`) 10, asset-URL helper 5, `fetch` 1, literal 5 |
| response type recorded | 490 of 502 (the `Promise<T>` of the enclosing method); 12 methods have no annotation |
| error handling | 385 `throw error?.response?.data` (UI sees the body only), 79 `throw error?.response` (UI sees status + body), 30 try/catch |

Per call site the CSV records: method, normalised path template(s), inline query, `params:` keys (or the declared type when it is a
variable), body type, `Promise<T>`, `.then` cast, what `.catch` throws, request options (`headers`, `validateStatus`, `responseType`) and
the service-method liveness evidence. `${this.serviceType}` is expanded from `EIssueServiceType` read out of `packages/types`
(`issues`, `epics`, `work-items`), which is why one site can carry three paths.

**Scan accuracy (the brief asked for a documented precision/recall estimate):**

| instrument | result |
|---|---|
| call-site recall: independent `this\.(get\|post\|put\|patch\|delete\|request)\(` regex over raw text vs extracted | **502 / 502** [computed] |
| literal recall: independent regex for every quoted `/api/...` or `/auth/...` literal (498 occurrences, 262 distinct normalised paths) vs every extracted path | **261 / 262 = 99.6%**; the miss is `issue.service.ts:261`, a template whose `${...}` contains quotes (the *regex* cannot parse it; the scanner did extract that call) |
| precision: 30 seeded random call sites (seed 9, from the 492 literal/derived ones) read against the source by hand, field by field | **30 / 30 correct** on verb, path, query, body, response, plus 29 / 29 on the catch shape (1 is try/catch, n/a). Rule-of-three upper bound on the error rate at 95% confidence: about 10% [computed]. The hand check was done by the author, not independently; verdicts are in `scripts/compat-inventory/handcheck-verdicts.csv` with the reasons for the notable rows |
| path precision for the 10 hand-resolved sites | by construction (each override cites the code it was read from) |

**Service-method liveness (is a method ever called?)** is a heuristic and the only part of the scan that is not exact [inferred]. Three
evidence levels, in `method_use_evidence`: *strict* (352 sites: the method name appears as `.method` in a file that mentions the class, a
subclass, or imports its module), *loose* (66: any `<something>Service.method` elsewhere; covers property inheritance such as a store base
class holding `this.cycleService`), *none* (84 sites, 79 distinct class.member). Loose can over-count on generic names (`list`, `retrieve`),
so a method can be wrongly called live, never wrongly dead except through destructuring or a receiver not named `*service*`. Validation: all
79 "none" members were checked by bare-name grep; 52 have no other occurrence at all, and for the 27 that do I read the hits (store actions
of the same name that call `rootStore.favorite.addFavorite(...)` instead, the generic `user-favorites/` endpoint, `patchState` instead
of `updateState`, ...) and found no live use of the *service* method. `used_by_apps` is attributed from strict evidence only; loose evidence
is recorded as `?` (used, app unknown). For methods of the shared `packages/services` classes with only loose evidence, the apps that import the class
(`class_consumer_apps`) are used; after that fallback no row is left with `?` only. This is how `InstanceWorkspaceService` (admin only), `Sites*Service`
(space only) and `APITokenService` (web only) were attributed; I verified those three by grep.

## 5. Join (step 3)

Join key: normalised path (`{}` placeholder per path parameter, trailing slash ignored), segment by segment; a server `{}` accepts any
client segment, the best match is the one with the most exactly-equal static segments (Django's first-match order breaks ties), the verb
must exist on the route. A client path with a placeholder where the server has a static segment never matches (it is reported).

| measure | value |
|---|---|
| call sites that target the Plane API (axios-style + non-axios refs) | 529 (5 more are external: presigned upload URL x4, GitHub API) |
| matched to a server route + verb | **478** |
| unmatched | **51** = 29 dead service methods + **22 live-looking** (table below) |
| path alternatives | 646: 500 matched; 52 `no-route`, 1 `method-mismatch`, 93 `variant-fanout` (the `epics/` and `work-items/` branches of `${this.serviceType}`; this server serves only `issues/`) |
| matched calls whose path has **no trailing slash** | 21 (rely on Django `APPEND_SLASH`; GET `users/notifications`, `advance-analytics*`, `cycles/{}/analytics`, `export-issues`, `project-identifiers`, `stickies/{}`, DELETE of assets / api-tokens / stickies) |
| ambiguous matches (two server routes tie) | 0 |

**Unmatched call sites that look live** (the route does not exist on this server, so the oracle answers 404; parity means reproducing the
404, not implementing them) [T6]:

| group | sites |
|---|---|
| EE / removed product areas (dashboards, integrations, Slack, GitHub, AI rephrase) | `dashboard.service.ts:19,35,53`, `packages/services/.../dashboard.service.ts:47,63`, `integration.service.ts:19,27,35`, `app_installation.service.ts:46,61`, `project.service.ts:143,154`, `select-repository.tsx:37`, `ai.service.ts:44` |
| paths that differ from the server (candidate real defects of this fork, reachable from the UI if the feature is used) | `PATCH sidebar-preferences/{key}/` (`workspace.service.ts:390`; the server only has `sidebar-preferences/`), `DELETE issue-relation/{id}/` (`issue.service.ts:195`; the server removes relations with `POST remove-relation/`), `POST bulk-operation-issues/` (`issue.service.ts:340`), `GET views/{id}/issues/` (`view.service.ts:59`), `POST assets/v2/workspaces/{slug}/{entity}/bulk/` (`file.service.ts:127`; the server has the project-level bulk only), `POST pages/{id}/move/` and `.../versions/{id}/restore/` |
| live collab server | `apps/live/.../core.service.ts:140` `GET pages/{id}/mentions/` |

**Server rows with no client use** (candidate `unused` / `drop`): **204** of 545, by surface: internal 171 (146 never called by any client
code, 25 reachable only through a dead service method), auth 12, instance 7, space 14. Internal by method: GET 73, POST 35, DELETE 32,
PATCH 18, **PUT 13 (all 13 PUT rows)**. By primary feature the biggest groups are F-D04 (22 asset routes: legacy `file-assets` and `user-assets`
variants), F-B18 (21: per-entity `user-favorite-*` routes, because the UI uses the generic `user-favorites/`), F-B14 (15: `intakes` /
`inboxes` / `intake-issues` aliases, the UI uses `inbox-issues`), F-B11 (13), F-A08 (11), F-A09 (11), F-B06 (12), F-B10 (9), F-D07 (9).
Caveat: "unused" means unused by *this React tree*; other consumers of the session API (Plane's mobile app, scripts) and `apps/live` internals
are not scanned [unverified]. The full list is `endpoint-inventory.csv` rows with `disposition = not-used`.

**Used by more than one client** (named apps): 21 routes: the page and description routes (web + live), `users/me` (web + live + space),
`PATCH users/me` and `users/me/profile` (web + space), `GET /api/instances/` (web, space, admin), asset DELETE/restore (web + space),
`auth/get-csrf-token` (web, space, admin), the five OAuth initiators (web + space). 137 routes are reached through the shared `packages/services` layer, so they are exposed to every app that imports
it. Space calls the public surface through `sites-*.service.ts` and uses `/auth/google|github|gitlab|gitea|oidc/` (non-`spaces/`) for OAuth,
so all `/auth/spaces/<provider>*` routes are unused.

## 6. Tagging (step 4)

Every server row carries `features` (FEATURES.md ids), `tier` (read from FEATURES.md at run time, so a tier edit flows through), `tag_note`
(why) and `call_sites` (which client call sites reach it): the evidence is in the same row. `disposition` = the tier when the route is
used; `not-used` when no client reaches it; `admin-only` when only `apps/admin` does; `v1-out-of-scope` for `/api/v1` (ADR-0031, a separate
surface; tagged `F-A5`); `drop` only for the 3 schema routes (FEATURES.md has no row that drops an internal route: the dormant integrations
have no server routes left, only dead client code).

| surface | P0 | P1 | P2 | drop | admin-only | not-used | infra | v1 | total |
|---|---|---|---|---|---|---|---|---|---|
| internal | 249 | 21 | 0 | 0 | 0 | 171 | 0 | 0 | 441 |
| space | 28 | 0 | 0 | 0 | 0 | 14 | 0 | 0 | 42 |
| auth | 13 | 8 | 8 | 0 | 0 | 12 | 0 | 0 | 41 |
| instance | 1 | 0 | 0 | 0 | 13 | 7 | 0 | 0 | 21 |
| v1 / web / schema | 0 | 0 | 0 | 3 | 0 | 0 | 2 | 132 | 137 |

Per primary feature: `tables.md` T10 (for example F-D09 published boards 32 P0 rows, F-D04 assets 21, F-B10 cycles 17, F-B11 modules 16, F-B12
views 16, F-B03 work items 13). Notes that affect planning:
- **P1 features on the P0 path.** `GET project-stats/` (F-D07 analytics, P1) fires from `fetchProjectDetails` on every project page, and
  `GET users/me/workspaces/invitations/` (F-A07 invites are P1) fires on entering onboarding. Both must exist at P0 and can return
  empty/zero shapes.
- **F-A10 (instance admin, P0 settings subset).** Only `GET /api/instances/` is used outside `apps/admin`; the other 13 used rows are
  admin-only (sign-in/up/out, admins, configurations, instance workspaces, email check): with `apps/admin` retired they are not compat-plane work.
  But `is_setup_done: false` makes the stock web app render `InstanceNotReady` and stop (`instance-wrapper.tsx:44`): **the first-user /
  instance bootstrap that god-mode performs today needs a replacement** (a seeded instance row is enough for the compat adapter).
- **OAuth is P2 but P0-shaped.** The 8 P2 auth rows are 4 provider initiators + 4 callbacks; they cost nothing until F-A05 is scheduled.
- Rule-based tagging cannot see intent: F-D09 (published boards) accounts for 32 P0 rows (the `/api/public/` surface used by `apps/space` plus
  `project-deploy-boards`), P0 only because the feature row is P0; `apps/space` needs SSR (section 7.7).

## 7. Shape catalogue (step 5)

All `file:line` below are in the pinned-pre tree; `shapes-anchors.json` re-extracts them (a moved anchor shows a new line, a vanished one is
`found: false`).

### 7.1 Pagination envelope and cursor (`apps/api/plane/utils/paginator.py`)

- **Cursor string** `"{value}:{offset}:{is_prev}"` (`Cursor.__str__` :30; parse `from_string` :48, three colon-separated parts, `value` int or
  float, otherwise 400 `{"detail": "Invalid cursor parameter."}`). The client sends `cursor=<per_page>:<page>:0` (`issue-filter-helper.store.ts:307`;
  with a group id the page is `1`): `offset` is a **page number**, not a row offset (:143).
- **`per_page`** (`BasePaginator.get_per_page` :642): default **1000**, cap 1000, over the cap is 400 `{"detail": "Invalid per_page value. Cannot exceed 1000."}`.
  Only **2 of 49** `paginate()` call sites override the default (`default_per_page=20`): a client that sends no `per_page` gets up to 1000 rows.
- **Envelope**, 12 keys, always (:716-731): `grouped_by`, `sub_grouped_by`, `total_count`, `next_cursor`, `prev_cursor`, `next_page_results`,
  `prev_page_results`, `count`, `total_pages`, `total_results`, `extra_stats`, `results`. `total_count == total_results == hits`; `count` is the
  size of this page; `total_pages = ceil(hits / limit)` for flat lists (`max_hits`; for grouped lists it is the largest group's count over the limit,
  :281-286); the paginator fetches `limit + 1` rows to compute `next_page_results` (:144).
  Ordering is the requested key, nulls last, then `-created_at` as tiebreak (:135-139).
- **Flat list:** `results` is an array. **Grouped** (`GroupedOffsetPaginator` :194, default per-group limit 50 :223): `results` is an object
  `{"<group value>": {"results": [...], "total_results": n}}` built with a `row_number()` window per group (:250). **Sub-grouped**
  (:389, default 30 :424): `{"<group>": {"results": {"<subgroup>": {"results": [...], "total_results": n}}, "total_results": n}}`.
- **Group keys** (`utils/grouper.py:144`): ids of states (non-triage), labels, modules, cycles, members; the literal string `"None"` is an extra
  group for labels / modules / cycles; `priority` is the fixed list low/medium/high/urgent/none; `state__group` the fixed five. m2m group fields are
  renamed by `FIELD_MAPPER` (:196, :391): `labels__id -> label_ids`, `assignees__id -> assignee_ids`, `issue_module__module_id -> module_ids`, and
  an item is **repeated in every group it belongs to**.
- **Item shape** in listings: `issue_on_results` (`grouper.py:93`, `required_fields` :106) returns a fixed set of 23 columns (`id`, `name`,
  `state_id`, `sort_order`, `completed_at`, `estimate_point`, `priority`, `start_date`, `target_date`, `sequence_id`, `project_id`, `parent_id`,
  `cycle_id`, `sub_issues_count`, `created_at`, `updated_at`, `created_by`, `updated_by`, `attachment_count`, `link_count`, `is_draft`,
  `archived_at`, `state__group`) plus `assignee_ids`, `label_ids`, `module_ids` (the grouped one is renamed as above).
- `group_by == sub_group_by` is 400 `{"error": "Group by and sub group by cannot have same parameters"}` (`issue/base.py:312`).
- **Which endpoints:** 25 compat rows call `paginate()` and 11 mention `group_by`; 29 GET rows are in either set (10 classed grouped-listing, 19
  paginated-listing). The 10 grouped rows: project issue list, `issues/list/`, sub-issues, cycle issues, archived issues, workspace user issues, the space
  issue list, module issues (never called), and the two analytics `*-charts` endpoints (they use a `group_by` argument for chart axes, P1). 12 `paginate()`
  sites pass a grouped paginator (10 in `plane.app`, 2 in `plane.space`): the group and sub-group branches of the project issue list
  (`issue/base.py:320,356`), cycle issues (`cycle/issue.py:153,188`), module issues (`module/issue.py:136,173`), archived issues
  (`issue/archive.py:150,185`), the workspace user-profile issues (`workspace/user.py:183,217`), and space (`space/views/issue.py:148,181`).

### 7.2 Query parameters (issue listings)

- Server reads `group_by`, `sub_group_by`, `order_by` (`-created_at` default), `cursor`, `per_page`, `updated_at__gt`, `expand`, `fields`, the rich
  `filters` JSON (`ComplexFilterBackend`, `utils/filters/filter_backend.py:20`, param name :28, max depth 5 :29, applied by `self.filter_queryset`
  in `IssueViewSet.list` `issue/base.py:253` **in addition to** the legacy filters), and **25 legacy params** (`issue_filters.py`: `assignees`,
  `completed_at`, `created_at`, `created_by`, `cycle`, `estimate_point`, `inbox_status`, `intake_status`, `labels`, `logged_by`, `mentions`, `module`,
  `name`, `parent`, `priority`, `project`, `start_date`, `start_target_date`, `state`, `state_group`, `sub_issue`, `subscriber`, `target_date`, `type`,
  `updated_at`).
- The client declares 26 (`TIssueParams`): 15 are legacy filter names, the other 11 are `cursor`, `expand`, `filters`, `group_by`, `issue_type`,
  `layout`, `order_by`, `per_page`, `show_empty_groups`, `sub_group_by`, `team_project`. Of these, `group_by`, `sub_group_by`, `order_by`, `cursor`,
  `per_page`, `filters` and `expand` are consumed by the views, the paginator or the rich-filter backend rather than `issue_filters`; `issue_type`,
  `show_empty_groups` and `team_project` appear in no server view as a query parameter (grep). The store really sends
  `group_by`, `sub_group_by`, `order_by`, `sub_issue`, `type`, `layout`, `cursor`, `per_page`, `target_date` and
  `filters=JSON.stringify(richFilters)` (`issue-filter-helper.store.ts:101-123, 307-340`). Ten legacy server params are never listed by the client
  (`completed_at`, `created_at`, `estimate_point`, `inbox_status`, `intake_status`, `logged_by`, `name`, `parent`, `start_target_date`, `updated_at`). Group names are translated by `EIssueGroupByToServerOptions`
  (`packages/constants/src/issue/common.ts:27`): `state -> state_id`, `labels -> labels__id`, `assignees -> assignees__id`, `module ->
  issue_module__module_id`, `cycle -> cycle_id`, `state_detail.group -> state__group`, `project -> project_id`.
- **Default display filters decide which listing runs first.** `get_default_display_filters()` (`db/models/issue.py:62`, `cycle.py:30`, ...) is
  `group_by: null, layout: "list", order_by: "-created_at", sub_issue: true`: the **first** listing a fresh project makes is the **flat** one
  [inferred from the defaults + the store; the dynamic half must confirm]. Grouped and sub-grouped listings start when the user picks Kanban or
  a grouping. Page sizes the UI uses: list 100 (50 grouped), kanban 30 (10 with sub-groups), spreadsheet/gantt 100, calendar 4/30.

### 7.3 `expand` / `fields`

`BaseViewSet.fields` / `.expand` (`app/views/base.py:139,144`; `BaseAPIView` :229,234) split a comma-separated query value; 12 server views use
`expand`, 13 use `fields`. Client values found: `issue_reactions,issue_attachments,issue_link,parent` (2 sites), `issue_inbox` (1),
`issue_relation,issue_related` (Gantt layout, `issue-filter-helper.store.ts:123`); `IssueService.getIssuesFromServer` switches the **path** to
`issues-detail/` when `expand` contains `issue_relation` and there is no `group_by` (`issue.service.ts:46-48`).

### 7.4 Error bodies

Four shapes coexist; the React app reads the body (385 methods) or the whole response (79):

| shape | where | note |
|---|---|---|
| `{"error": "<text>"}` | 185 `Response({"error": ...})` sites in `plane.app` + the catch-all in `BaseViewSet.handle_exception` / `BaseAPIView.handle_exception` (`base.py:70-110`, :167-210) | IntegrityError -> 400 "The payload is not valid"; ValidationError -> 400 "Please provide valid detail"; ObjectDoesNotExist -> 404 "The required object does not exist."; KeyError -> 400 "The required key does not exist."; anything else -> 500 "Something went wrong please try again later" |
| `{"detail": "<text>"}` | DRF `ParseError` / `NotAuthenticated` / `PermissionDenied`, through `EXCEPTION_HANDLER = auth_exception_handler` (`settings/common.py:130`) | `NotAuthenticated` is forced to **401** (`authentication/adapter/exception.py:20-22`); `Throttled` becomes **429** with `{"error_code": 5900, "error_message": "RATE_LIMIT_EXCEEDED"}` (:24-29) |
| `{"<field>": ["<code or text>", ...]}` | 49 `Response(serializer.errors)` sites | for example project create returns `name: ["PROJECT_NAME_ALREADY_EXIST"]`, which the form reads with `errorData.name?.includes(...)` (`ce/components/projects/create/root.tsx`) |
| `{"message": "<text>"}` | 29 sites | |

Throttle rates `anon: 30/minute`, `asset_id: 5/minute` (`common.py:123-126`).

### 7.5 Cookies, CSRF, session

- **Session cookie** `session-id` (`SESSION_COOKIE_NAME`, `common.py:358`), HttpOnly (:355), `Secure` from the origin config (:354), 604800 s (:357), domain
  `COOKIE_DOMAIN` (:359), **database-backed** engine `plane.db.models.session` (:356); admin cookie `admin-session-id`, 3600 s (:363-364).
- **CSRF cookie** `csrftoken` (Django's default name; not overridden [inferred]), **HttpOnly** (:368), `Secure` (:367), trusted origins = CORS origins (:369),
  failure view `authentication.views.common.csrf_failure` (:371). Because it is HttpOnly, JS cannot read it: `GET /auth/get-csrf-token/` returns
  `{"csrf_token": "..."}` (`views/common.py:28-35`) and the SPA copies it into a hidden `csrfmiddlewaretoken` input (11 sites) or an `X-CSRFTOKEN`
  header (4 call sites: `auth.service.ts` web and package, `user.service.ts:157,270`).
- **DRF endpoints skip CSRF** (`BaseSessionAuthentication.enforce_csrf` returns `None`, `authentication/session.py:10-11`); only the Django-`View`
  `/auth/*` POSTs go through `CsrfViewMiddleware` (`common.py:109`). `CORS_ALLOW_CREDENTIALS = True` (:160), extra allowed header `X-API-Key` (:171).
- **Unauthenticated** = 401 + `{"detail": ...}`. The web `APIService` response interceptor turns **any** 401 into
  `window.location.replace('/?next_path=<pathname>')` (`apps/web/core/services/api.service.ts:26-35`); `packages/services`' `APIService` has no
  interceptor. `validateStatus: null` is used by `UserService.currentUser` (web, `GET /api/users/me/`), and by the package's `InstanceService.info`,
  `.admins` and `AuthService.requestCSRFToken` (space/admin), so for those the body, not the status, decides; the web app's own `getInstanceInfo`
  has no override (non-2xx becomes `MaintenanceView`).

### 7.6 The `/auth/*` server-redirect flow

41 routes, one `View`/`APIView` class per route (not DRF viewsets), 133 `HttpResponseRedirect(` call sites and 19 `next_path` reads across
`authentication/views`. Sign-in/up are **HTML form posts** (`application/x-www-form-urlencoded`: `email`,
`password`, `next_path`, `csrfmiddlewaretoken`; magic code adds `code`) submitted by the browser to `${API_BASE_URL}/auth/sign-in|sign-up/`
(`password.tsx:151`), never XHR. Every outcome is a **302**:
- failure: `base_host(request, is_app=True)` (`authentication/utils/host.py:16`, `WEB_URL` or `APP_BASE_URL`) + `next_path` + query
  `error_code` (59 codes, `authentication/adapter/error.py:5`) + `error_message` + payload such as `email` (`get_error_dict` :95), built by
  `get_safe_redirect_url` (`utils/path_validator.py:142`, which also rejects hostile paths). React reads `error_code` from the URL
  (`auth-root.tsx:41`, `reset-password.tsx:49`) and maps it with `authErrorHandler`.
- success: `user_login()` (`authentication/utils/login.py:14`) = Django `login()` (sets `session-id`) + `device_info` in the session, then redirect to
  `next_path` or `get_redirection_path(user)` (`utils/redirection_path.py:7`): `onboarding` -> last workspace slug -> first workspace slug ->
  `invitations` -> `create-workspace`.
- an instance without `is_setup_done` answers every sign-in/up with `INSTANCE_NOT_CONFIGURED` (5000) (`views/app/email.py:33-47`).
- OAuth / OIDC: the browser navigates (`window.location.assign('${API_BASE_URL}/auth/google/?next_path=...')`, `hooks/oauth/core.tsx:46,62,71,80,90`),
  the provider returns to `/auth/<provider>/callback/`, which 302s the same way. Sign-out is a dynamically built form POST with the CSRF token
  (`apps/web/core/services/auth.service.ts:72`).
- JSON endpoints in the same prefix: `email-check` (POST), `magic-generate`, `forgot-password`, `set-password` (needs `X-CSRFTOKEN`),
  `change-password`, `get-csrf-token`.

### 7.7 Other wire behaviours

- `GET /api/users/me/` and `/api/users/me/settings/`: `Cache-Control: private, max-age=12` + `Vary: Cookie` (`user/base.py:74-81`); the client busts
  the cache with `?t=<Date.now()>` (`user.service.ts:96`).
- `GET /api/instances/`: server-side response cache 2 h (`cache_response(60 * 60 * 2, user=False)`, `license/api/views/instance.py:34`) invalidated by 10
  admin handlers. `cache_response` is on 6 compat rows in total (4 shared, 2 per-user) and `invalidate_cache(path=...)` on 23, the writers of the
  cached lists (workspaces, my-workspaces, labels, states, estimates), which gives read-your-writes. `GzipMiddleware` is global (`common.py:113`); `gzip_page` is
  also applied to 8 list handlers.
- **Asset reads are 302 redirects to a presigned URL**: `WorkspaceFileAssetEndpoint.get`, `ProjectAssetEndpoint.get`, `StaticFileAssetEndpoint.get`,
  the two `*DownloadEndpoint.get` and space `EntityAssetEndpoint.get` (`app/views/asset/v2.py:414,442,612,802,829`; `space/views/asset.py:35`). Images are
  plain `<img src="${API_BASE_URL}/api/assets/v2/...">` (`getFileURL`, `packages/utils/src/file.ts`), so the browser sends the session cookie.
- **Comments are listed through the activity endpoint**: `GET issues/{id}/history/?activity_type=issue-comment` (`issue_comment.service.ts:36`,
  `epic-comment` for epics). The `/comments/` list route exists but is never called; create/update/delete use `/comments/` (`PATCH`, never `PUT`).
- Trailing slashes: server routes all end with `/`; 21 matched calls omit it (section 5). `CommonMiddleware` is installed (`common.py:108`),
  `APPEND_SLASH` is unset, so Django's default (redirect) applies [inferred].
- **Presigned upload (F-D04):** `POST assets/v2/workspaces/{slug}/` returns `{asset_id, upload_data: {url, fields...}, asset_url}`; the browser then
  POSTs (or PUTs, `getFileUploadRequestOptions`) the file **directly to `upload_data.url`** with `withCredentials: false`
  (`file-upload.service.ts:30-51`), then `PATCH assets/v2/workspaces/{slug}/{asset_id}/` confirms. The object store must be reachable from the browser.
- `apps/live` (collab) forwards the user's `Cookie` header to `GET /api/users/me/` and the page endpoints
  (`apps/live/src/services/user.service.ts:21`, `page/project-page.service.ts:29`); it reads/writes `pages/{id}/description/` (binary) and follows
  the 302 of `assets/v2/...` with `maxRedirects: 0` to read the `Location` header (`page/core.service.ts:173`).
- **Creating a workspace seeds it.** `POST /api/workspaces/` dispatches `workspace_seed.delay(...)` (`app/views/workspace/base.py:139`;
  `bgtasks/workspace_seed_task.py:504`), which creates a **bot user** (`bot_user_<workspace id>@plane.so`, `is_bot`, workspace admin), a **sample
  project named after the workspace** with members, states, labels and work items, and `track_event` fires too (:141). So on the stock path the first
  `GET workspaces/{slug}/projects/` and `members/` are **not empty**, and the member list contains a bot. Whether Contrail reproduces the seed is a
  parity decision (a deliberate deviation if not), because the oracle diff will see it.
- `apps/space` is **server-rendered** (`react-router.config.ts`: `ssr: true`; a `loader` calls `GET /api/public/anchor/{anchor}/meta/` from the server,
  `app/issues/[anchor]/layout.tsx:42`), while `apps/web` and `apps/admin` are SPAs (`ssr: false`).

## 8. Effort classes and sizing (step 6)

**Scope of a person-day:** the compat HTTP adapter only (axum routing, session extraction, request parsing, response/serializer shaping, Plane error
bodies, oracle-diff fixes) over core use-cases that exist anyway for F-B01..F-B16. It excludes the core use-cases, the schema, the identity subsystem and the
outbox. **Classification is computed; unit costs are my judgement [inferred] and live in `effort_model.py`.** The low / high columns multiply the marginal
cost of each class by 0.7x / 1.6x (grouped listing 0.7x / 2.0x, aggregate 0.6x / 2.0x) and the shared machinery by 0.7x / 1.6x.

| class | computed rule | pd per row (S / M / L) | what the cost is |
|---|---|---|---|
| crud | default: one resource, fewer than 3 aggregation calls | 0.15 / 0.40 / 0.90 | route + extractor + core call + serializer field mapping; M/L are mutating handlers that also emit background events and map error codes |
| paginated-listing | GET, handler calls `paginate()`, no `group_by` | 0.5 / 0.8 / 1.2 | cursor parse, ordering, envelope (after the envelope machinery exists) |
| grouped-listing | GET with `group_by` | 1.0 / 1.5 / 2.5 | group/sub-group result shaping (after the group-by engine exists) |
| bulk | path rule (`bulk-*`, `issue-dates`, `transfer-issues`, `cycle-issues` POST, module issue adds, `mark-all-read`) | 0.4 / 0.8 / 1.2 | multi-row writes plus per-row activity events |
| file-upload | `assets/`, `file-assets/`, `attachments/` paths | 0.4 / 0.8 / 1.2 | after the presigned flow exists |
| auth-redirect | the `/auth/*` surface | 0.3 / 0.8 / 1.5 | after the csrf + session + 302 machinery exists |
| aggregate-other | 3+ aggregation calls (`annotate`, `Subquery`, `Window`, ...) or analytics/search/export/history/stats paths | 0.4 / 1.0 / 2.5 | SQL aggregation parity |

Size buckets come from the handler AST: S = at most 25 lines and 4 ORM calls, M = at most 70 lines, L = more; handlers inherited from DRF defaults are S.

**Shared machinery** (the part a per-row count hides), pd [inferred]: route scaffold + authn/permission seam 3.0; error bodies 1.0; pagination envelope 2.0;
**group-by engine 6.0**; issue-filter machinery (legacy params + rich `filters` tree) 6.0; `expand`/`fields` projection 2.0; csrf + session + form-redirect flow 4.0;
presigned asset flow 3.0; oracle-diff iteration over ~25 hot serializers 3.0. Total 30.0.

| scope | rows | paths | marginal (likely) | + machinery | **total pd low / likely / high** |
|---|---|---|---|---|---|
| every compat row (internal + space + instance + auth) | 545 | 313 | 297.6 | 30.0 | 219.0 / **327.6** / 574.6 |
| used by React and not dropped (P0 + P1 + P2) | 328 | 230 | 201.3 | 30.0 | 154.9 / **231.3** / 406.3 |
| **P0 rows used by React** | 291 | 198 | 177.0 | 30.0 | 138.4 / **207.0** / 364.0 |
| P0 minus the minimum slice | 245 | 173 | 150.7 | 17.5 | 112.0 / **168.2** / 298.1 |
| **minimum P0 slice** | 48 | 42 | 26.1 | 12.5 | 26.2 / **38.6** / 65.6 |

P0 rows by class (291 rows, marginal likely, pd): crud 185 rows 48.8; aggregate-other 35 rows 64.7; file-upload 30 rows 16.8; grouped-listing 7 rows 17.5;
paginated-listing 13 rows 10.0; auth-redirect 13 rows 12.4; bulk 8 rows 6.8. Full per-class tables for every scope: `tables.md` T7.

### 8.1 The minimum P0 slice (exact endpoints)

Journey through the unmodified UI: instance info -> sign up -> onboarding (profile, workspace) -> workspace home -> create project (default cover upload)
-> open project -> create work item -> see it in the list. Each row names the trigger; `scripts/compat-inventory/p0_slice.py` is validated against the
join on every run. `kind = min`: needed; `ext`: needed only to open the created item.

| id | stage | kind | method | server path (positional) | class / size | pd | features | triggered at |
|---|---|---|---|---|---|---|---|---|
| S01 | boot | min | GET | /api/instances/ | crud/L | 0.9 | F-A10 | apps/web/core/lib/wrappers/instance-wrapper.tsx:24 |
| S02 | boot | min | GET | /api/users/me/ | crud/S | 0.15 | F-A09,F-A03 | apps/web/core/lib/wrappers/authentication-wrapper.tsx:41 |
| S03 | auth | min | GET | /auth/get-csrf-token/ | auth-redirect/S | 0.3 | F-A03 | apps/web/core/components/account/auth-forms/password.tsx:74 |
| S04 | auth | min | POST | /auth/email-check/ | auth-redirect/M | 0.8 | F-A03 | apps/web/core/components/account/auth-forms/form-root.tsx:57 |
| S05 | auth | min | POST | /auth/sign-up/ | auth-redirect/L | 1.5 | F-A03 | apps/web/core/components/account/auth-forms/password.tsx:151 |
| S06 | auth | min | POST | /auth/sign-in/ | auth-redirect/L | 1.5 | F-A03 | apps/web/core/components/account/auth-forms/password.tsx:151 |
| S07 | boot | min | GET | /api/users/me/profile/ | crud/S | 0.15 | F-A09 | apps/web/core/store/user/index.ts:121 |
| S08 | boot | min | GET | /api/users/me/settings/ | crud/S | 0.15 | F-A09 | apps/web/core/store/user/index.ts:122 |
| S09 | boot | min | GET | /api/users/me/workspaces/ | aggregate-other/M | 1.0 | F-B01 | apps/web/core/store/user/index.ts:123 |
| S10 | onboarding | min | PATCH | /api/users/me/ | crud/S | 0.15 | F-A09,F-A03 | apps/web/core/components/onboarding/steps/profile/root.tsx:96 |
| S11 | onboarding | min | PATCH | /api/users/me/profile/ | crud/S | 0.15 | F-A09 | apps/web/core/components/onboarding/steps/workspace/create.tsx:109 |
| S12 | onboarding | min | GET | /api/users/me/workspaces/invitations/ | crud/S | 0.15 | F-A07 (P1) | apps/web/app/(all)/onboarding/page.tsx:43 |
| S13 | onboarding | min | GET | /api/workspace-slug-check/ | crud/S | 0.15 | F-B01 | apps/web/core/components/onboarding/steps/workspace/create.tsx:75 |
| S14 | onboarding | min | POST | /api/workspaces/ | crud/L | 0.9 | F-B01 | apps/web/core/components/onboarding/steps/workspace/create.tsx:79 |
| S15 | workspace-load | min | GET | /api/workspaces/{}/workspace-members/me/ | aggregate-other/M | 1.0 | F-A07 | apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:76 |
| S16 | workspace-load | min | GET | /api/users/me/workspaces/{}/project-roles/ | crud/S | 0.15 | F-A08 | apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:81 |
| S17 | workspace-load | min | GET | /api/workspaces/{}/projects/ | aggregate-other/L | 2.5 | F-B02 | apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:88 |
| S18 | workspace-load | min | GET | /api/workspaces/{}/members/ | crud/S | 0.15 | F-A07 | apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:94 |
| S19 | workspace-load | min | GET | /api/workspaces/{}/user-favorites/ | crud/S | 0.15 | F-B18 | apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:100 |
| S20 | workspace-load | min | GET | /api/workspaces/{}/states/ | crud/S | 0.15 | F-B04 | apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:110 |
| S21 | workspace-load | min | GET | /api/workspaces/{}/sidebar-preferences/ | crud/M | 0.4 | F-B18 | apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:117 |
| S22 | workspace-load | min | GET | /api/workspaces/{}/user-properties/ | crud/S | 0.15 | F-A09 | apps/web/core/layouts/auth-layout/workspace-wrapper.tsx:124 |
| S23 | workspace-load | min | GET | /api/workspaces/{}/users/notifications/unread/ | crud/M | 0.4 | F-D01 | apps/web/core/components/workspace-notifications/notification-app-sidebar-option.tsx:29 |
| S24 | home | min | GET | /api/workspaces/{}/home-preferences/ | crud/M | 0.4 | F-B18 | apps/web/core/components/home/root.tsx:29 |
| S25 | home | min | GET | /api/workspaces/{}/quick-links/ | crud/S | 0.15 | F-B18 | apps/web/core/components/home/widgets/links/root.tsx:32 |
| S26 | home | min | GET | /api/workspaces/{}/recent-visits/ | aggregate-other/S | 0.4 | F-B18 | apps/web/core/components/home/widgets/recents/index.tsx:48 |
| S27 | home | min | PATCH | /api/users/me/tour-completed/ | crud/S | 0.15 | F-A09 | apps/web/core/components/home/root.tsx:36 |
| S28 | project-create | min | POST | /api/assets/v2/workspaces/{}/ | file-upload/M | 0.8 | F-D04 | apps/web/ce/components/projects/create/root.tsx:74 |
| S29 | project-create | min | PATCH | /api/assets/v2/workspaces/{}/{}/ | file-upload/S | 0.4 | F-D04 | apps/web/core/services/file.service.ts:92 |
| S30 | project-create | min | POST | /api/workspaces/{}/projects/ | crud/M | 0.4 | F-B02 | apps/web/ce/components/projects/create/root.tsx:96 |
| S31 | project-create | min | POST | /api/assets/v2/workspaces/{}/projects/{}/{}/bulk/ | file-upload/M | 0.8 | F-D04 | apps/web/core/components/project/create-project-modal.tsx:57 |
| S32 | project-create | min | PATCH | /api/workspaces/{}/projects/{}/ | crud/M | 0.4 | F-B02 | apps/web/ce/components/projects/create/root.tsx:100 |
| S33 | project-create | min | GET | /api/assets/v2/workspaces/{}/{}/ | file-upload/S | 0.4 | F-D04 | packages/utils/src/editor/common.ts:26 |
| S34 | project-open | min | GET | /api/workspaces/{}/projects/{}/ | crud/M | 0.4 | F-B02 | apps/web/core/layouts/auth-layout/project-wrapper.tsx:83 |
| S35 | project-open | min | GET | /api/workspaces/{}/project-stats/ | aggregate-other/M | 1.0 | F-D07 (P1) | apps/web/core/store/project/project.store.ts:388 |
| S36 | project-open | min | GET | /api/workspaces/{}/projects/{}/project-members/me/ | crud/S | 0.15 | F-A08 | apps/web/core/layouts/auth-layout/project-wrapper.tsx:88 |
| S37 | project-open | min | GET | /api/workspaces/{}/projects/{}/user-properties/ | crud/S | 0.15 | F-B12 | apps/web/core/layouts/auth-layout/project-wrapper.tsx:90 |
| S38 | project-open | min | GET | /api/workspaces/{}/projects/{}/issue-labels/ | crud/S | 0.15 | F-B05 | apps/web/core/layouts/auth-layout/project-wrapper.tsx:96 |
| S39 | project-open | min | GET | /api/workspaces/{}/projects/{}/members/ | crud/S | 0.15 | F-A08 | apps/web/core/layouts/auth-layout/project-wrapper.tsx:101 |
| S40 | project-open | min | GET | /api/workspaces/{}/projects/{}/states/ | crud/S | 0.15 | F-B04 | apps/web/core/layouts/auth-layout/project-wrapper.tsx:106 |
| S41 | project-open | min | GET | /api/workspaces/{}/projects/{}/intake-state/ | crud/S | 0.15 | F-B14 | apps/web/core/layouts/auth-layout/project-wrapper.tsx:111 |
| S42 | project-open | min | GET | /api/workspaces/{}/projects/{}/estimates/ | crud/S | 0.15 | F-B13 | apps/web/core/layouts/auth-layout/project-wrapper.tsx:116 |
| S43 | project-open | min | GET | /api/workspaces/{}/projects/{}/cycles/ | crud/L | 0.9 | F-B10 | apps/web/core/layouts/auth-layout/project-wrapper.tsx:121 |
| S44 | project-open | min | GET | /api/workspaces/{}/modules/ | aggregate-other/L | 2.5 | F-B11 | apps/web/core/layouts/auth-layout/project-wrapper.tsx:129 |
| S45 | project-open | min | GET | /api/workspaces/{}/projects/{}/modules/ | crud/M | 0.4 | F-B11 | apps/web/core/layouts/auth-layout/project-wrapper.tsx:129 |
| S46 | project-open | min | GET | /api/workspaces/{}/projects/{}/views/ | crud/M | 0.4 | F-B12 | apps/web/core/layouts/auth-layout/project-wrapper.tsx:134 |
| S47 | work-item | min | GET | /api/workspaces/{}/projects/{}/issues/ | grouped-listing/L (x0.5, flat branch only) | 1.25 | F-B03,F-B16 | apps/web/core/store/issue/project/issue.store.ts:117 |
| S48 | work-item | min | POST | /api/workspaces/{}/projects/{}/issues/ | crud/L | 0.9 | F-B03,F-B16 | apps/web/core/store/issue/helpers/base-issues.store.ts:534 |
| X01 | work-item | ext | GET | /api/workspaces/{}/projects/{}/issues/{}/ | aggregate-other/L | 2.5 | F-B03 | apps/web/core/components/issues/peek-overview/root.tsx:220 |
| X02 | work-item | ext | GET | /api/workspaces/{}/projects/{}/issues/{}/history/ | aggregate-other/M | 1.0 | F-B08 | apps/web/core/services/issue/issue_comment.service.ts:36 |
| X03 | work-item | ext | PATCH | /api/workspaces/{}/projects/{}/issues/{}/ | aggregate-other/L | 2.5 | F-B03 | apps/web/core/services/issue/issue.service.ts:227 |

Not in the slice but one click away: `POST /auth/sign-out/` (a dynamic form post), `GET /api/timezones/` (settings), the notification inbox, every
page-level route. The slice shows **35 GET, 8 POST, 5 PATCH**. Seven slice handlers dispatch a Celery task today: S14 (`workspace_seed` and `track_event`,
see 7.7), S29 (`get_asset_object_metadata`), S30 (`model_activity`), S32, S34 and S47 (`recent_visited_task`, i.e. even reads write), S48 (issue activity and
friends), so the outbox is on the slice's write path as well, and two of the three extension rows (X01, X03) dispatch too.

Machinery in the slice (pd): route scaffold 3.0, error bodies 1.0, csrf/session/redirect 3.0 (0.75 of 4.0), presigned assets 1.8 (0.6 of 3.0), pagination 1.0
(0.5: flat envelope only), filter parser 1.5 (0.25: `order_by`, `sub_issue`, `type`, `layout`, `filters={}`), serializer fidelity 1.2 (0.4: about 10 of 25 hot
serializers). **Excluded** from the slice and therefore the first follow-ons: the group-by engine (6.0) and the `expand`/`fields` projection (2.0). The fractions are
judgement [inferred].

### 8.2 What the owner needs for Q-0009.1 (the cap)

| quantity | pd likely (low-high) |
|---|---|
| minimum slice (the "can one user do anything" gate) | 38.6 (26-66) |
| everything React uses at P0 | 207.0 (138-364), 5.4x the slice |
| ... of which the `/auth/*` 302 emulation (13 P0 rows + the csrf/session machinery) | about 16 (12.4 marginal + 4.0 machinery) [inferred] |
| ... of which the presigned asset flow (30 P0 rows + 3.0) | about 20 [inferred] |
| ... of which group-by / filter / pagination / expand machinery (16.0) + 7 grouped + 13 paginated rows (27.5) | about 44 [inferred] |

This is data, not a recommendation: whether 207 pd breaches the cap, and whether to reopen ADR-0034 (adapt the React service layer), is the owner's call.
The auth client is the one clear case where a Q-R2-allowed change replaces emulation (section 9).

## 9. React behaviours that depend on endpoint quirks, and the allowed area that would be cheaper

Q-R2 allows: base URL / service layer, auth client, collab provider, branding. Costs to emulate are [inferred]; counts are computed.

| # | behaviour in the React tree | where | emulate in the adapter | change in an allowed area |
|---|---|---|---|---|
| 1 | sign-in/up/magic/reset/OAuth are browser **form posts and navigations** answered by 302 with `?error_code=`/`next_path`; CSRF token fetched as JSON because the cookie is HttpOnly | 13 files, 21 reference sites: web `auth-forms/{password,unique-code,reset-password}.tsx`, `hooks/oauth/core.tsx`, `services/auth.service.ts`; space `auth-forms/{password,unique-code}.tsx`, `hooks/oauth/core.tsx`, `navbar/user-avatar.tsx`; `packages/services/.../auth.service.ts`; 3 in `apps/admin` | 41 rows, csrf/session/redirect machinery, 59 error codes, `get_redirection_path`, `next_path` safety: about 16 pd for the P0 part | **auth client**: JSON POSTs to the identity adapter, return the same `error_code` values in a JSON body so `authErrorHandler` is unchanged, set the cookie from the response |
| 2 | any 401 triggers `window.location.replace('/?next_path=...')` | `apps/web/core/services/api.service.ts:26-35` | the adapter must return **401 exactly for "no/expired session"** and 403 for role failures; mixing them logs users out or hides forbidden states | **service layer**: scope the interceptor to session endpoints, or soft-navigate |
| 3 | auth state is decided from the **body** of `GET /api/users/me/` (`validateStatus: null`), instance state from the body of `GET /api/instances/` | `user.service.ts:57`, `instance-wrapper.tsx` | unauthenticated `users/me` must be a JSON object without `id` (not HTML, not a redirect); `instances` must carry `is_setup_done: true` | none needed; keep as is |
| 4 | 21 calls omit the trailing slash and rely on Django's redirect | section 5 | accept both forms (trivial) | **service layer**: normalise in `APIService` |
| 5 | comments are read from `history/?activity_type=issue-comment` | `issue_comment.service.ts:36` | implement the multiplexed activity endpoint with `activity_type` filtering and `epic-comment` | **service layer**: point `getIssueComments` at a native comments list if the core API has one |
| 6 | project creation uploads a default **static cover** through presign -> browser POST to the object store -> confirm -> bulk attach -> PATCH | `ce/components/projects/create/root.tsx:60-100`, `core/services/file.service.ts:72-95,134-147`, `file-upload.service.ts` | 4 asset routes + an object store reachable and CORS-open to the browser | **service layer**: `FileService.uploadWorkspaceAsset` could do one multipart POST to the Worker; changing the default cover (`getProjectFormValues`) is a component edit outside the Q-R2 list |
| 7 | images are `<img src="${API_BASE_URL}/api/assets/v2/...">` answered by 302 to a presigned URL | `packages/utils/src/file.ts`, `editor/common.ts:24-41` | 302 or stream, with the session cookie | **base URL**: `getFileURL` could target a signed-cache route |
| 8 | the collab server authenticates by forwarding the user's `Cookie` to `GET /api/users/me/` and reads/writes page description binaries | `apps/live/src/services/user.service.ts:21`, `page/core.service.ts` | `pages/{id}/description/` GET/PATCH (binary) stay needed until the DO collab provider lands | **collab provider** (ADR-0060) replaces this path |
| 9 | `users/me*` and `settings/` are cached 12 s privately; the client busts with `?t=` | `user.service.ts:96` | drop the headers (harmless) | none |
| 10 | `epics/` and `work-items/` fan-out (`EIssueServiceType`) and every EE area (dashboards, integrations, importers, Slack, GitHub) | 93 path alternatives, 22 live-looking call sites | do not implement; answer 404 like the oracle | none |
| 11 | `PUT` is never used (13 server rows); `inbox-issues`, not `intake-issues`/`intakes`/`inboxes`; `user-favorites/`, not `user-favorite-*` | section 5 | do not implement 171 internal rows | none |
| 12 | server redirects use the absolute `WEB_URL`/`APP_BASE_URL` | `authentication/utils/host.py:16` | relative redirects are enough in the same-origin topology (ADR-0011) | none |

## 10. Top 5 quirks that bite

1. **Auth is a browser-form + 302 protocol, not an API.** HttpOnly CSRF cookie fetched as JSON, hidden-field POSTs, every outcome a 302 whose query
   carries `error_code` (59 codes) and `next_path`, success redirects computed by `get_redirection_path`; unauthenticated `users/me` must be a body without
   `id`, and a stray 401 anywhere else hard-redirects the SPA (sections 7.5, 7.6).
2. **The issue listing contract.** 12-key envelope, cursor `per_page:page:is_prev` where the middle part is a page number, three result shapes (array,
   dict of groups, dict of dicts), m2m groups renamed and duplicated, a `"None"` group, per-group limits 50/30, default `per_page` 1000, rich `filters`
   JSON plus 25 legacy params (section 7.1, 7.2). It is only 1 of the 48 slice rows on the first fetch but the heaviest follow-on (about 44 pd with the
   grouped and paginated rows, section 8.2).
3. **The stock UI drags in the presigned asset flow on first project creation**, and needs a browser-reachable object store plus 302-redirect images (section 7.7, 8.1).
4. **Every page fires a wrapper full of reads**: 9 per workspace page, 13 per project page, including P1-tier ones (`project-stats`, `intake-state`).
   An "empty" project still needs about 46 shape-correct routes (section 8.1).
5. **The surface is much smaller and odder than the route table suggests**: 39% of internal rows are never called; the UI reads comments through
   `history/`, patches with `PATCH` only, uses the `inbox-issues` alias and the generic `user-favorites/`; 22 live-looking calls hit routes this server does
   not have and the oracle will 404 them (sections 5, 9).

## 11. Not in scope here, and what the dynamic half must verify

Out of scope for this half, by design: HAR recording, call frequency from real traffic, the replay harness. They wait on the spike-0008 harness (pinned
instance + recorder). **Frequency ranking in `endpoint-inventory.csv` is therefore a static proxy only** (slice membership, number of client call sites,
shared-layer exposure); no number here is a request count.

The dynamic half is a **static-vs-dynamic diff**. Against the recorded scripted journeys it must verify:
1. **Slice completeness**: every request in the HAR for the journey is in `p0_slice.py`; extra requests are slice gaps, slice rows that never fire are
   over-claims. Use `endpoint-inventory.csv` `call_sites` to map each HAR request to its static call site.
2. **Static-only vs dynamic-only routes**: 204 `not-used` rows that *do* appear in a journey; the 22 live-looking unmatched call sites (do they fire? what does
   the UI do with the 404?); the 66 call sites whose liveness rests on loose evidence; whether `epics/` or `work-items/` variants are ever sent.
3. **Response bodies**: field sets, nullability, date formats, enum values, empty-state shapes for a fresh workspace/project on S01-S48; this desk half did
   not read serializers' output, only counted them (`serializer_fields` column).
4. **Listing behaviour**: the first issue listing of a new project (flat vs grouped, section 7.2 inference), the exact query strings including
   `filters=` and `layout=`, per-group `total_results`, `"None"` groups, `next_page_results`, behaviour at `per_page` 0/negative/over cap.
5. **Status codes and error bodies per failure**: 401 vs 403 vs 404 (JSON or HTML?), 400 shapes per endpoint, the 429 body, CSRF failure response.
6. **Cookies and redirects**: cookie names, flags and rotation on login, `Set-Cookie` presence per response; the `Location` value for every `/auth/*` outcome,
   `next_path` safety rules, the error code per case; magic-code and reset flows.
7. **Presigned flow**: `upload_data` shape (POST fields vs PUT headers) on the instance's object store, `asset_url` format, the 302 targets, the `asset_id`
   throttle (5/minute, `common.py:125`).
8. **Headers**: `Cache-Control`/`Vary` on `users/me`, gzip, `Content-Type`, `Content-Length` for large lists, CORS preflight with `X-API-Key`.
9. **Live collab calls**: which `apps/live` requests and which cookie forwarding happen on a real page edit; the WebSocket auth handshake.
10. **The woven OIDC routes** on the pinned commit.
11. **Workspace seeding**: what `workspace_seed` actually creates on the pinned instance (bot user, sample project, work items), because it changes the
    first `projects/`, `members/`, `issues/` responses of every fresh workspace.
12. **Throttles / rate limits** on auth endpoints (`authentication/rate_limit.py`): only anchored here, not catalogued [unverified].
13. Call **ordering and revalidation** (SWR `revalidateOnFocus`, duplicate fetches) and cache hit behaviour.

## 12. Notes for the question rows (proposed text only; no plan file other than this one was edited)

- **Q-0010.1** (which internal endpoints and shapes React depends on): desk answer = `out/endpoint-inventory.csv` (545 compat rows, 291 P0-used) and sections 7-8;
  closes when the dynamic half has confirmed bodies (section 11 items 1-4). Proposed F-X13 acceptance list: the 291 `disposition = P0` rows, ordered slice first.
- **Q-0009.1** (cap): data in section 8.2. Slice 38.6 pd (26-66); P0 compat 207.0 pd (138-364).
- **Q-0011.1** (three apps under one Workers asset namespace): static inputs only [build output not examined, unverified]. API calls use an **origin-only**
  `API_BASE_URL` (default `""` = same origin) and absolute `/api/...`, `/auth/...` paths (`packages/constants/src/endpoints.ts`); web is an SPA at `/`
  (`ssr: false`, no `basename`); admin an SPA under `VITE_ADMIN_BASE_PATH` and space under `VITE_SPACE_BASE_PATH` (both `basename` and Vite `base`); **space is
  `ssr: true`** with a server loader that fetches the API (`apps/space/app/issues/[anchor]/layout.tsx:42`), so it cannot be served as plain static assets; server
  redirects build the app/space/admin origins and paths through `base_host()` (`authentication/utils/host.py:16-52`, fallbacks `/spaces/` and `/god-mode/`). Only 7 of the 502 axios call sites are server-side (`apps/live`); `apps/space` adds one SSR `fetch`.
- **Q-R2** (minimal React change): section 9 lists which of the allowed areas pay for themselves; row 1 (auth client) and rows 2, 4, 5, 6 (service layer) are the
  candidates; the default project cover (row 6) is outside the allowed list.

## 13. Limits and register of non-computed claims

[inferred]: all person-day unit costs and slice fractions; the liveness heuristic (and the claim that `PUT` rows are dead for every consumer); "first listing is flat";
the Django `csrftoken` cookie name and `APPEND_SLASH` default; that `/auth/spaces/<provider>` is unused by design; epics/work-items variants only being reached by EE
components (the enum is used in CE components behind `isEpic`).
[unverified]: other consumers of the session API (mobile app, scripts); runtime response bodies; throttle behaviour; build output and asset layout; DRF's default text of
the 401 body; whether the 22 live-looking unmatched calls are reachable in the pinned build.
Known scan limits: services reached through receivers not named `*service*` and called by destructuring are not seen by the liveness heuristic; JSX text with an apostrophe
can hide tokens on that line for the non-axios literal scan (recall was still 99.6%); `apps/live`'s per-request base path is hand-substituted
(`PLACEHOLDER_SUBST`); the `AnalyticsService.processUrl` and two asset-delete paths are hand-resolved (all 10 overrides cite their source).

## 14. Files

All under `.agents/plans/contrail/`, uncommitted:
- `research/0003-compat-plane-inventory.md` (this file)
- `scripts/compat-inventory/`: `run_all.py`, `layout.py`, `verify_api_tree.py`, `server_routes.py`, `pyindex.py`, `client_calls.py`, `ts_lex.py`, `client_overrides.py`, `join_tag.py`,
  `feature_rules.py`, `effort_model.py`, `p0_slice.py`, `effort_report.py`, `shapes_probe.py`, `precision_recall.py`, `compare_runs.py`, `render_tables.py`,
  `handcheck-verdicts.csv` (pre-pin), `handcheck-verdicts-pinned.csv` (pinned: 30 rows, all ok, re-read against the pinned files)
- `scripts/compat-inventory/out/` (pre-pin, commit `300c2009d4`): `endpoint-inventory.csv + .json`, `server-routes.csv + .json`, `server-route-entries.json`, `server-summary.json`, `client-calls.csv + .json`,
  `client-refs.csv + .json`, `client-scan-stats.json`, `join-matches.csv`, `join-report.json`, `join-unmatched.json`, `effort-summary.json`, `shapes-anchors.json`,
  `precision-recall.json`, `handcheck-sample.csv`, `tables.md`, `RUN.json`, `server-warnings.txt`, `client-unresolved.json`, `layout-report.json`
- `scripts/compat-inventory/out-pinned/` (pinned, commit `ba85d4d3d3`): the same file set, plus `delta-vs-prepin.json` (`compare_runs.py --old out --new out-pinned`, section 15)
  and `pin-verify.json` (`verify_api_tree.py`)

## 15. Post-pin re-run (pinned `ba85d4d3d3`)

**Answer first.** The same scripts ran on the pinned tree after being made layout-tolerant (15.10). **No conclusion in sections 1-14 is invalidated** (15.11).
The route tables are identical except five new `/api/v1` `-lite` routes; the React tree lost one call and re-slashed two; the 22 live-looking 404 calls are the same 22;
the four woven OIDC routes survived byte-identical. The minimum slice is still 48 rows / 42 paths and moves **38.6 -> 39.3 pd**, P0 compat **207.0 -> 207.6 pd**, both from
handler-size bucket crossings. What did change is **behaviour behind unchanged routes**: **123 of the 545 compat rows (77 of 290 P0) have a changed handler body**, mostly upstream
security hardening (tenant scoping, role tightening, `order_by`/`group_by` allowlists, auth throttling). The pre-pin text describes a more permissive server; the oracle is the
pinned one (15.3, 15.6, 15.11).

### 15.1 What was run, and what the numbers describe

| | pre-pin (`out/`) | pinned (`out-pinned/`) |
|---|---|---|
| tree | docs worktree `.../worktrees/docs-plane-on-cloudflare-spec` | `.../worktrees/agent-a69ad036a9c59b0a8` (read-only; **git was never run against it**) |
| commit | `300c2009d4fdcf5369b92e108da0c2de4212eb4a` (`--vcs git`) | worktree HEAD `72b2e014972cc30b38d56290da501c56af866c65` (`--vcs files`: the `.git` pointer is read as a file). `oracle/PIN`: `pinned_commit=ba85d4d3d332b5380bebca4b9e0143b1127de907`, `pin_tag=oracle/v1-pin-20261018`, `fork_base_commit=300c2009d4...`, `upstream_sha=879ce8b5c6b4...` (`v1.4.2-rc1-111-g879ce8b5c6`, 2026-10-17) |
| TS/TSX files scanned | 3,283 | 2,595 (15.10) |
| delta | | `uv run compare_runs.py --old out --new out-pinned --json out-pinned/delta-vs-prepin.json` |

- **The server numbers are the pin's.** `verify_api_tree.py` recomputed the git tree sha of `apps/api` from the working files (no git): `5827f6f67f5c00fc7815332808c89bb9d6080914`, equal to `oracle/PIN`
  `apps_api_tree` (`out-pinned/pin-verify.json`, `match: true`) [computed].
- **The client numbers are "the client code in that worktree".** `oracle/PIN` records no tree hash for `apps/web`, `apps/space`, `apps/admin`, `apps/live` or `packages/*`, and HEAD is a descendant of the pin
  (oracle-harness commits, which the PIN header says must not touch `apps/api`), so equality of the client dirs with the pin commit cannot be shown without git [unverified].
- **Pre-pin reproducibility.** Re-running the layout-tolerant scripts on the pre-pin tree gives an `out/` byte-identical to the one first delivered, except (a) the new `layout-report.json` and (b) four
  slice trigger line numbers in `effort-summary.json` / `tables.md`: S07 `store/user/index.ts` 111 -> 121, S08 111 -> 122, S09 111 -> 123, S28 `projects/create/root.tsx` 66 -> 74. The new
  trigger-token check showed those four were wrong in the first deliverable; the 8.1 table above is corrected. No count, class or day changed. Both runs are deterministic (two runs, `diff -r` clean).
- **Instruments on the pinned tree.** Call-site recall 501/501; literal recall 260/261 = 99.6% (the same nested-template miss, `issue.service.ts:261`); the seeded 30-site hand check was re-read against the
  pinned files (`handcheck-verdicts-pinned.csv`): 30/30 on verb, path, query, body and response, 29/29 on catch shape. Slice validation: 0 errors (all 51 entries exist in the join; every trigger token is
  still present; only three trigger lines drifted: S02 41 -> 50, S05 and S06 151 -> 143). Shape anchors: 41 checked, 0 violated.

### 15.2 Route counts per surface, pre-pin -> pinned

| surface | path() entries | method x path rows | distinct paths |
|---|---|---|---|
| internal (`/api/`, `plane.app`) | 233 -> 233 | 441 -> 441 | 233 -> 233 |
| space (`/api/public/`) | 25 -> 25 | 42 -> 42 | 25 -> 25 |
| auth (`/auth/`) | 41 -> 41 | 41 -> 41 | 41 -> 41 |
| **v1 (`/api/v1/`)** | 68 -> **73** | 132 -> **137** (GET 56 -> 61) | 67 -> **72** |
| instance (`/api/instances/`, `plane.license`) | 14 -> 14 | 21 -> 21 | 14 -> 14 |
| web (`/`) / schema (conditional) | 2 / 3 -> 2 / 3 | 2 / 3 -> 2 / 3 | unchanged |
| **total** | 386 -> **391** | 682 -> **687** | |

The 545 compat rows (internal + space + instance + auth) are unchanged in number; internal rows by method are unchanged (DELETE 78, GET 186, PATCH 60, POST 104, PUT 13). Reconciliation: `api/urls/*.py` raw
`path(` lines 70 -> 75, reachable 68 -> 73 (`estimate.py` and `schema.py` are still never included; the two `include(router.urls)` still expand to 3 entries each); every other conf file is unchanged.
Handler binding: plain-`APIView` rows 410 -> 415 (the five new v1 rows); DRF-default rows 77 -> 76 (compat 73 -> 72: `PATCH /api/workspaces/{}/invitations/{}/` now has an explicit handler); compat rows that
dispatch a Celery task **98 -> 95** (P0 **73 -> 72**): five handlers lost a `.delay(` call, three of them now have none (`DELETE /api/workspaces/{}/`, `POST /api/users/me/workspaces/invitations/`,
`POST /api/workspaces/{}/invitations/{}/join/`) and two went from 2 tasks to 1 (`POST /api/workspaces/`, `POST /api/workspaces/{}/invitations/`); consistent with the removed `track_event` telemetry calls (read in `workspace/base.py` only) [inferred].

### 15.3 Added, removed and changed endpoints

**Added (5 rows, all `/api/v1`, out of scope per ADR-0031, all GET and paginated):** `/api/v1/workspaces/{}/members-lite/`, `/api/v1/workspaces/{}/projects-lite/`,
`/api/v1/workspaces/{}/projects/{}/cycles-lite/`, `/api/v1/workspaces/{}/projects/{}/modules-lite/`, `/api/v1/workspaces/{}/projects/{}/project-members-lite/`.
**Removed: none**, on any surface.

**Changed.** Four levels, from most to least visible to the scripts.

1. **Contract fields the script compares (23 rows)** (`server_contract_changed`):
   - 8 auth POSTs gained `@throttle_auth_redirect`: `/auth/sign-in/`, `/auth/sign-up/`, `/auth/magic-sign-in/`, `/auth/magic-sign-up/` and the four `/auth/spaces/...` equivalents (15.7, quirk 1).
   - Legacy file-asset routes, permission `[IsAuthenticated]` -> `[IsAuthenticated, WorkspaceMemberPermission]` (7 rows; two are P0, the editor's asset delete and restore, the other five not-used): `DELETE|GET|POST /api/workspaces/{}/file-assets/`,
     `DELETE (P0)|GET|POST /api/workspaces/file-assets/{}/{}/`, `POST /api/workspaces/file-assets/{}/{}/restore/` (P0).
   - `GET /api/workspaces/{}/entity-search/`: `[IsAuthenticated]` -> `WorkspaceUserPermission`.
   - `PATCH /api/workspaces/{}/projects/{}/states/{}/`: `allow_permission([ADMIN, MEMBER, GUEST])` -> `[ADMIN]` (state edits are admin-only now).
   - `GET /api/workspaces/{}/invitations/{}/join/` and `GET /api/workspaces/{}/projects/{}/join/{}/`: the public invitation lookup returns a field subset (`WorkSpaceMemberInvitePublicSerializer`, 10 fields; `ProjectMemberInvitePublicSerializer`, 7) instead of `__all__`.
   - `PATCH /api/workspaces/{}/invitations/{}/`: was a DRF default, now an explicit handler with `WorkSpaceMemberInviteSerializer`.
   - 3 v1 asset rows (`GET|PATCH /api/v1/workspaces/{}/assets/{}/`, `POST .../assets/`): `[IsAuthenticated]` -> `[WorkspaceUserPermission]`.
2. **Serializer shape (5 rows)**: the invitation-join pair and the PATCH above, plus `GET /api/workspaces/{}/members/` and `.../members/{}/` (`WorkspaceMemberAdminSerializer` `Meta.fields` `__all__` -> not declared, probably an `exclude` [inferred]).
3. **Tier / effort classification (24 rows)**: 15.6.
4. **Handler source text (123 compat rows = 95 internal, 21 space, 4 auth, 3 instance; 97 distinct handlers; plus 29 v1 rows, not listed)** (`server_handler_body_changed`, new section of `compare_runs.py`: handler span text
   with blank and comment-only lines ignored). By tier: P0 77 of 290, P1 8, admin-only 2, not-used 36. The LOC-only counter the first version of the tool had saw 118 rows (all surfaces) and could not say what changed.
   Notation: `ws/` = `workspaces/{}/`, `pr/` = `projects/{}/`, paths are under `/api/` except auth; the tier is in parentheses. Read 13 of the diffs by hand (list in 15.12): they are real semantic changes, not reformatting.

   **internal (95)**
   - `app/views/analytic/base.py` (4): GET ws/analytics/ (not-used); GET ws/default-analytics/ (not-used); GET ws/saved-analytic-view/{}/ (not-used); POST ws/export-analytics/ (not-used)
   - `app/views/api.py` (4): DELETE users/api-tokens/ (not-used); DELETE users/api-tokens/{}/ (P0); GET users/api-tokens/ (P0); GET users/api-tokens/{}/ (P0)
   - `app/views/asset/base.py` (2): POST workspaces/file-assets/{}/{}/ (not-used); POST ws/file-assets/ (not-used)
   - `app/views/asset/v2.py` (11): DELETE assets/v2/ws/ (not-used); DELETE assets/v2/ws/{}/ (P0); GET assets/v2/static/{}/ (not-used); GET assets/v2/ws/ (not-used); GET assets/v2/ws/{}/ (P0); PATCH assets/v2/ws/ (not-used); PATCH assets/v2/ws/{}/ (P0); POST assets/v2/ws/ (P0); POST assets/v2/ws/duplicate-assets/{}/ (P0); POST assets/v2/ws/pr/{}/bulk/ (P0); POST assets/v2/ws/{}/ (not-used)
   - `app/views/cycle/archive.py` (6): GET ws/pr/archived-cycles/ (P0); GET ws/pr/archived-cycles/{}/ (P0); GET ws/pr/cycles/{}/archive/ (not-used); POST ws/pr/archived-cycles/ (not-used); POST ws/pr/archived-cycles/{}/ (not-used); POST ws/pr/cycles/{}/archive/ (P0)
   - `app/views/cycle/base.py` (1): GET ws/pr/cycles/{}/analytics/ (P0)
   - `app/views/cycle/issue.py` (1): POST ws/pr/cycles/{}/cycle-issues/ (P0)
   - `app/views/estimate/base.py` (3): DELETE ws/pr/estimates/{}/estimate-points/{}/ (not-used); PATCH ws/pr/estimates/{}/ (not-used); POST ws/pr/estimates/{}/estimate-points/ (P0)
   - `app/views/intake/base.py` (2): GET ws/pr/inbox-issues/ (P0); GET ws/pr/intake-issues/ (not-used)
   - `app/views/issue/activity.py` (1): GET ws/pr/issues/{}/history/ (P0)
   - `app/views/issue/attachment.py` (6): DELETE assets/v2/ws/pr/issues/{}/attachments/ (not-used); DELETE assets/v2/ws/pr/issues/{}/attachments/{}/ (P0); GET assets/v2/ws/pr/issues/{}/attachments/ (P0); GET assets/v2/ws/pr/issues/{}/attachments/{}/ (not-used); PATCH assets/v2/ws/pr/issues/{}/attachments/ (not-used); PATCH assets/v2/ws/pr/issues/{}/attachments/{}/ (P0)
   - `app/views/issue/base.py` (2): DELETE ws/pr/bulk-delete-issues/ (P0); GET ws/pr/issues/list/ (P0)
   - `app/views/issue/comment.py` (1): POST ws/pr/issues/{}/comments/ (P0)
   - `app/views/issue/link.py` (1): PATCH ws/pr/issues/{}/issue-links/{}/ (P0)
   - `app/views/issue/relation.py` (3): GET ws/pr/issues/{}/issue-relation/ (P0); POST ws/pr/issues/{}/issue-relation/ (P0); POST ws/pr/issues/{}/remove-relation/ (P0)
   - `app/views/issue/sub_issue.py` (2): GET ws/pr/issues/{}/sub-issues/ (P0); POST ws/pr/issues/{}/sub-issues/ (P0)
   - `app/views/module/archive.py` (3): GET ws/pr/archived-modules/ (P0); GET ws/pr/archived-modules/{}/ (P0); GET ws/pr/modules/{}/archive/ (not-used)
   - `app/views/module/base.py` (1): GET ws/pr/modules/{}/ (P0)
   - `app/views/module/issue.py` (2): DELETE ws/pr/modules/{}/issues/{}/ (P0); POST ws/pr/modules/{}/issues/ (P0)
   - `app/views/notification/base.py` (1): GET ws/users/notifications/ (P0)
   - `app/views/page/base.py` (1): GET ws/pr/pages/{}/ (P0)
   - `app/views/page/version.py` (2): GET ws/pr/pages/{}/versions/ (P1); GET ws/pr/pages/{}/versions/{}/ (P1)
   - `app/views/project/base.py` (3): GET ws/projects/details/ (P0); PATCH ws/pr/ (P0); POST ws/pr/project-deploy-boards/ (P0)
   - `app/views/project/invite.py` (3): GET ws/pr/join/{}/ (not-used); POST users/me/ws/projects/invitations/ (P1); POST ws/pr/join/{}/ (not-used)
   - `app/views/project/member.py` (1): PATCH ws/pr/members/{}/ (P0)
   - `app/views/search/base.py` (1): GET ws/entity-search/ (P0)
   - `app/views/state/base.py` (3): PATCH ws/pr/states/{}/ (P0); POST ws/pr/states/ (P0); POST ws/pr/states/{}/mark-default/ (P0)
   - `app/views/user/base.py` (1): GET users/me/activities/ (not-used)
   - `app/views/view/base.py` (2, DRF-default rows compared on `get_queryset`): POST ws/views/ (P0); PUT ws/views/{}/ (not-used)
   - `app/views/webhook/base.py` (5): PATCH ws/webhooks/ (not-used); PATCH ws/webhooks/{}/ (P0); POST ws/webhooks/ (P0); POST ws/webhooks/{}/ (not-used); POST ws/webhooks/{}/regenerate/ (P0)
   - `app/views/workspace/base.py` (2): DELETE ws/ (P0); POST workspaces/ (P0)
   - `app/views/workspace/cycle.py` (1): GET ws/cycles/ (P0)
   - `app/views/workspace/draft.py` (1): POST ws/draft-to-issue/{}/ (P0)
   - `app/views/workspace/invite.py` (5): GET ws/invitations/{}/join/ (P1); PATCH ws/invitations/{}/ (P1); POST users/me/workspaces/invitations/ (P1); POST ws/invitations/ (P1); POST ws/invitations/{}/join/ (P1)
   - `app/views/workspace/member.py` (3): DELETE ws/members/{}/ (P0); PATCH ws/members/{}/ (P0); POST ws/members/leave/ (P0)
   - `app/views/workspace/module.py` (1): GET ws/modules/ (P0)
   - `app/views/workspace/user.py` (2): GET ws/user-activity/{}/ (P0); GET ws/user-profile/{}/ (P0)
   - `app/views/workspace/user_preference.py` (1): PATCH ws/sidebar-preferences/ (P0)

   **space (21)**
   - `space/views/asset.py` (7): GET public/assets/v2/anchor/{}/ (not-used); GET public/assets/v2/anchor/{}/{}/ (P0); PATCH public/assets/v2/anchor/{}/ (not-used); PATCH public/assets/v2/anchor/{}/{}/ (P0); POST public/assets/v2/anchor/{}/ (P0); POST public/assets/v2/anchor/{}/restore/{}/ (P0); POST public/assets/v2/anchor/{}/{}/ (not-used)
   - `space/views/intake.py` (2): POST public/anchor/{}/intakes/{}/inbox-issues/ (not-used); POST public/anchor/{}/intakes/{}/intake-issues/ (not-used)
   - `space/views/issue.py` (12, the 3 GET rows on `comments/` and `reactions/` are DRF-default rows compared on `get_queryset`): DELETE public/anchor/{}/comments/{}/reactions/{}/ (P0); DELETE public/anchor/{}/issues/{}/comments/{}/ (P0); DELETE public/anchor/{}/issues/{}/reactions/{}/ (P0); GET public/anchor/{}/comments/{}/reactions/ (not-used); GET public/anchor/{}/issues/{}/ (P0); GET public/anchor/{}/issues/{}/comments/ (P0); GET public/anchor/{}/issues/{}/comments/{}/ (not-used); PATCH public/anchor/{}/issues/{}/comments/{}/ (P0); POST public/anchor/{}/comments/{}/reactions/ (P0); POST public/anchor/{}/issues/{}/comments/ (P0); POST public/anchor/{}/issues/{}/reactions/ (P0); POST public/anchor/{}/issues/{}/votes/ (P0)

   **auth (4)**: POST /auth/reset-password/{}/{}/ (P0, `authentication/views/app/password_management.py`); POST /auth/change-password/ (P0) and POST /auth/set-password/ (P0) (`views/common.py`); POST /auth/spaces/reset-password/{}/{}/ (not-used).
   **instance (3)**: GET instances/ (P0, `license/api/views/instance.py`); POST instances/admins/sign-in/ and POST instances/admins/sign-up/ (admin-only, `license/api/views/admin.py`).

   What the diffs show **[diff]**, by theme (13 handlers read): tenant scoping added to lookups that trusted a bare URL id (`Project.objects.get(pk=pk, workspace__slug=slug)`, issue bound to workspace + project in
   `POST .../comments/` with a new 404 `{"error": "Issue not found"}`, `CycleIssue` and `Issue` lookups scoped in `cycle-issues`, project existence check in `project-deploy-boards`); new 403 branches on assets
   (15.6); `order_by` sanitised to an allowlist on projects, notifications and user activity; module list restricted to the caller's projects; webhook create and regenerate now pass `show_secret_key: True`
   (so other responses presumably omit the secret; the serializer is unread [unverified]); the PostHog fields and `track_event` calls removed. Several changes carry GHSA ids in the code comments.

   **Shared code behind the rows, which the row-level scripts do not see [diff]:**
   - `utils/order_queryset.py` (new allowlists) + `sanitize_order_by`: an unknown `order_by` is **silently replaced by `-created_at`** on issues (`order_issue_queryset`, hence the S47 listing), intake issues,
     activity, projects, views, notifications and pages (18 call sites, v1 included). `utils/paginator.py`: `group_by` / `sub_group_by` must be in `ISSUE_GROUP_BY_ALLOWLIST` or the listing answers **400
     `ParseError` ("Invalid group_by field: ...")**.
   - `utils/issue_filters.py`: the `updated_at` issue filter now filters `updated_at__date` (pre-pin it filtered `created_at__date`).
   - `app/permissions/workspace.py`: `WorkSpaceBasePermission` PUT/PATCH now `role=Admin` only (was Admin or Member), `WorkspaceOwnerPermission` requires `is_active`, new `WorkspaceMemberPermission`;
     `app/permissions/project.py`: the POST branch requires a `ProjectMember` of the URL project (was any workspace member). The permission *class names* in the table T9 are the same, their behaviour is not: the permission seam (F-E00b) must be written from the pinned bodies.
   - `authentication/adapter/base.py` `complete_login_or_signup` (every provider): deactivated accounts rejected, bot accounts rejected with error code 5017, IdP-sync condition corrected (`is_signup = not bool(user)`; pre-pin it was inverted, no effect for OIDC, which has no sync key), avatar download through an SSRF-safe pinned fetch (15.8).
   - Error bodies: `Response({"error": ...})` call sites 185 -> 209 and `Response(serializer.errors)` 49 -> 50, i.e. the new 403/404 branches use the existing `{"error": "..."}` shape; no new error shape (7.4 stands).
   - `GET .../issues/{}/history/` without `activity_type` (a branch the React tree never takes): both lists are now serialised before the merge sort; pre-pin it chained model instances and sorted by `instance["created_at"]`, which cannot have worked.

### 15.4 Client call sites

| measure | pre-pin | pinned |
|---|---|---|
| `APIService` subclasses | 71 | 71 |
| `this.<verb>(` call sites | 502 (web 341, pkg-services 154, live 7) | **501** (web 340) |
| verbs | GET 209, POST 140, DELETE 83, PATCH 66, PUT 4 | GET 208, rest unchanged |
| path resolution | 483 literal, 9 derived, 10 override, 0 unresolved | 482 / 9 / 10 / 0 |
| non-axios path references | 32 | 32, **the same set** (no ref added or removed) |
| method liveness evidence (strict / loose / none) | 352 / 66 / 84 | 351 / 66 / 84 |
| files scanned | 3,283 | 2,595 |

**Changed call sites, complete list (1 removed, 2 re-pathed):**
- removed: `UserService.getUserActivity`, GET `/api/users/me/activities/` (apps/web). The route is now `not-used` (was P0).
- re-pathed (a trailing slash was added): `WorkspaceNotificationService.fetchNotifications` (apps/web) and `WorkspaceNotificationService.list` (packages/services), GET `/api/workspaces/{slug}/users/notifications` -> `.../users/notifications/`.
- not a call site, found through the shape anchors: the 401 interceptor (`apps/web/services/api.service.ts`) now URL-encodes the redirect, `/?next_path=${encodeURIComponent(currentPath)}`.
- every other call site is identical in client, `Class.member`, verb and path template; only its file path moved (15.10) and a few lines shifted by one.

### 15.5 Join, pre-pin -> pinned

| measure | pre-pin | pinned |
|---|---|---|
| call sites that target the Plane API | 529 | 528 |
| matched to a server route + verb | 478 | 477 |
| unmatched | 51 = 29 dead + 22 live-looking | **51 = 29 dead + 22 live-looking (same sites)** |
| path alternatives (matched) | 646 (500) | 645 (499); `no-route` 52, `method-mismatch` 1, `variant-fanout` 93 in both |
| matched calls with no trailing slash | 21 | **19** (the two notification calls) |
| ambiguous matches | 0 | 0 |
| routes used by more than one client / reached through `packages/services` | 21 / 137 | 21 / 137 |
| compat server rows with no client use | 204 | **205** (internal 171 -> 172, GET 73 -> 74; auth 12, instance 7, space 14 unchanged) |
| ... internal: never called / only via a dead method | 146 / 25 | 147 / 25 |
| ... internal `PUT` rows unused | 13 of 13 | 13 of 13 |

Unused routes: the only change is **+1, `GET /api/users/me/activities/`** (P0 -> not-used; the F-A09 feature row goes from 18 P0 / 11 not-used to 17 / 12). Internal unused share 172 of 441 = **39.0%**.
Unmatched live-looking calls: no site added, none removed, none became matched (15.9).
Tiers (545 compat rows): P0 291 -> **290**, P1 29, P2 8, admin-only 13, not-used 204 -> **205**; internal P0 249 -> 248, not-used 171 -> 172. v1 132 -> 137 (out of scope).
P0 rows by class (marginal pd, likely): crud 185 -> 187 rows (48.8 -> 50.3), aggregate-other 35 -> 33 (64.7 -> 62.7), file-upload 30 -> 30 (16.8 -> 18.4), paginated-listing 13 -> 12 (10.0 -> 9.5), grouped-listing 7, auth-redirect 13, bulk 8 unchanged.

### 15.6 The minimum P0 slice and the estimates

**The slice is structurally unchanged**: still 48 rows / 42 distinct paths (+3 extension rows), 35 GET / 8 POST / 5 PATCH, 0 validation errors, no entry removed or added, the same 7 slice handlers dispatch a Celery task (S14 now dispatches 1 task, `workspace_seed`; it was 2).
Per-endpoint behaviour changes in the slice (9 of 51 handlers changed text; the pd column is the model's, same unit costs):

| id | endpoint | what changed in the pinned handler [diff] | size | pd |
|---|---|---|---|---|
| S01 | GET /api/instances/ | `posthog_api_key` / `posthog_host` and their config keys removed from the response (144 -> 130 lines); `is_oidc_enabled` kept. `packages/types/src/instance/base.ts` no longer declares them | L | 0.9 (same) |
| S14 | POST /api/workspaces/ | the `WORKSPACE_CREATED` `track_event` dispatch removed (`workspace_seed.delay` kept); 84 -> 70 lines | L -> M | 0.9 -> 0.4 |
| S28 | POST /api/assets/v2/workspaces/{}/ | `WORKSPACE_LOGO` uploads now 403 unless the caller is a workspace admin (64 -> 75 lines); the slice's `PROJECT_COVER` upload is unaffected | M -> L | 0.8 -> 1.2 |
| S29 | PATCH /api/assets/v2/workspaces/{}/{}/ | a project-bound asset needs an active `ProjectMember` of its project (403 "You don't have access to this asset."); workspace-level assets (`project_id` null, which a fresh project cover is) are exempt | S -> M | 0.4 -> 0.8 |
| S31 | POST .../assets/v2/workspaces/{}/projects/{}/{}/bulk/ | assets looked up only if `created_by` is the caller and unassociated or already in this project; others are silently left out, no error [inferred from the filter] (52 -> 63 lines) | M | 0.8 (same) |
| S32 | PATCH /api/workspaces/{}/projects/{}/ | project fetched by `pk` **and** workspace slug | M | 0.4 (same) |
| S33 | GET /api/assets/v2/workspaces/{}/{}/ | the same project-access 403; the 302 target is built with `filename`, `disposition="inline"` and `content_type` from the asset attributes (so the presigned URL's response-header parameters differ) | S -> M | 0.4 -> 0.8 |
| S44 | GET /api/workspaces/{}/modules/ | only modules of projects the caller is an active member of and that are not archived; `member_ids` annotated (90 -> 108 lines) | L | 2.5 (same) |
| X02 | GET .../issues/{}/history/ | see 15.3 (branch the UI does not take) | M | 1.0 (same) |

Trigger lines: 48 of 51 unchanged after the file moves; S02 41 -> 50, S05 and S06 151 -> 143.

**Estimates, same effort model** (same unit costs, same 30.0 pd of shared machinery, same S/M/L thresholds of 25 and 70 handler lines), pre-pin -> pinned, person-days likely (low-high):

| scope | rows | pre-pin | pinned |
|---|---|---|---|
| **minimum P0 slice** | 48 | 38.6 (26.2-65.6) | **39.3 (26.7-66.7)** |
| P0 minus the slice | 245 -> 244 | 168.2 (112.0-298.1) | 168.2 (112.2-297.2) |
| **P0 rows used by React** | 291 -> 290 | 207.0 (138.4-364.0) | **207.6 (139.1-364.2)** |
| used by React, not dropped (P0+P1+P2) | 328 -> 327 | 231.3 (154.9-406.3) | 233.2 (156.3-409.0) |
| every compat row | 545 | 327.6 (219.0-574.6) | 333.5 (223.2-583.8) |

**What moved** (the 24 `inventory_changes`; nothing else feeds the estimate): all from handler-size buckets crossing a threshold, two class changes and one disposition.
- **M -> L (9):** `DELETE .../estimates/{}/estimate-points/{}/`; `PATCH .../projects/{}/members/{}/`; `POST /api/assets/v2/workspaces/{}/` and `/{}/{}/` (S28); `POST .../projects/{}/join/{}/`; space `POST .../intakes/{}/inbox-issues/` and `/intake-issues/`; space `POST /api/public/assets/v2/anchor/{}/` and `/{}/{}/`.
- **S -> M (11, 2 of them v1):** `GET` and `PATCH /api/assets/v2/workspaces/{}/` and `/{}/{}/` (S29, S33); `GET .../pages/{}/versions/` and `/{}/`; space `DELETE .../comments/{}/`, `DELETE .../reactions/{}/`, `PATCH .../comments/{}/`; v1 `PATCH .../links/{}/` (issues and work-items).
- **L -> M (1):** `POST /api/workspaces/` (S14).
- **class aggregate-other -> crud (2):** `DELETE /api/workspaces/{}/members/{}/`, `POST /api/workspaces/{}/members/leave/` (fewer than 3 aggregation calls now).
- **disposition P0 -> not-used (1):** `GET /api/users/me/activities/`.

The minimum slice's +0.7 pd is exactly S14 -0.5, S28 +0.4, S29 +0.4, S33 +0.4 (marginal 26.1 -> 26.8; machinery 12.5 unchanged); P0 overall +0.6 = the slice's +0.7 and a net 0.0 for the rest of P0 (the removed activities row and the two class changes cancel the other crossings); the full-compat +5.9 comes from the
asset, space-asset and page-version crossings and the P1 / not-used rows (24 moves in all). Section 8.2 recomputed: P0 / slice 5.4x -> **5.3x**; asset flow 30 P0 rows + 3.0 = about 21 (was about 20); auth about 16 (13 rows, unchanged); group-by / filter / pagination machinery 16.0 + 7 grouped + 12 paginated rows (27.0) = about 43 (was 44).
**The move is 0.3% of the P0 total, far inside the model's own 0.7x-1.6x band; it does not change any answer to Q-0009.1.**
What the model does not carry: unit costs depend on class and size, not on how many authorisation branches a handler has, so the 77 changed P0 handlers add parity cases (403/404/silent-exclusion paths the oracle will show) that no pd line covers [inferred]; the replay harness, not this model, is where they get priced.

### 15.7 The five top quirks (section 10)

| # | quirk | pinned status |
|---|---|---|
| 1 | auth is a form post + 302 protocol | **Unchanged in kind, three details added.** (a) The 8 sign-in/sign-up POSTs (email and magic, app and space) are now actually rate-limited: `throttle_auth_redirect` applies `AuthenticationThrottle` (anonymous, **default 10/minute per IP**, `AUTHENTICATION_RATE_LIMIT`) and answers with the same 302 `?error_code=` (`RATE_LIMIT_EXCEEDED`). Pre-pin the magic views set `throttle_classes` on plain Django `View`s, where DRF throttling does not run (the pinned `rate_limit.py` says so), and the email views had none, so sign-in/up were effectively unthrottled [inferred from source]; S05 and S06 are in the slice, and **a replay harness doing more than 10 sign-ins a minute from one IP will see the 302 rate-limit outcome**. (b) 60 error codes, was 59: `BOT_USER_LOGIN_FORBIDDEN` (5017), returned by `complete_login_or_signup` for bot accounts on every provider. (c) the 401 interceptor URL-encodes `next_path`. Password strength still needs zxcvbn score >= 3, now scored on at most 128 characters from each end. |
| 2 | the issue listing contract | **Unchanged in kind.** Envelope, cursor, group shapes and the 25 legacy params are untouched (all paginator, filter and expand anchors match). Two additions: `order_by` is allowlisted (unknown -> silently `-created_at`) and `group_by` / `sub_group_by` are validated (unknown -> 400); the `updated_at` filter now uses `updated_at`. |
| 3 | the stock UI drags in the presigned asset flow | **Unchanged in kind, costlier.** Same four slice routes and the same browser POST; the asset handlers gained authorisation (S28/S29/S33 +1.2 pd in the model), the 302 target carries disposition / content-type parameters, and the bulk-attach only touches the caller's own uploads. |
| 4 | every page fires a wrapper of reads | **Unchanged.** `workspace-wrapper.tsx` (9 reads) and `project-wrapper.tsx` (13) differ only in imports and UI-component markup [diff]; all trigger tokens for S15-S25 and S34-S46 sit on the same lines. |
| 5 | the surface is smaller and odder than the route table | **Unchanged.** 172 of 441 internal rows never called (39.0%, was 171); the UI still reads comments through `history/` (X02 trigger unchanged); the same 22 live-looking calls hit routes the server does not have. |

Section 9 rows that moved by a number: row 1 "59 error codes" -> 60; row 2 `next_path` is now encoded; row 4 "21 calls omit the trailing slash" -> 19; row 11 "171 internal rows" -> 172. Rows 3, 5-10 and 12 stand.

### 15.8 The woven OIDC routes

- **All four routes survived, unchanged:** `GET /auth/oidc/`, `GET /auth/oidc/callback/`, `GET /auth/spaces/oidc/`, `GET /auth/spaces/oidc/callback/` (`OIDCOauthInitiateEndpoint`, `OIDCCallbackEndpoint`,
  `OIDCOauthInitiateSpaceEndpoint`, `OIDCCallbackSpaceEndpoint`). Tier P1; disposition P1 for the first two (used by web and space) and not-used for the two `/auth/spaces/oidc*`; effort class and size identical pre and pinned.
  `server_rows_removed` is empty.
- `provider/oauth/oidc.py`, `views/app/oidc.py`, `views/space/oidc.py` and `authentication/urls.py` are **byte-identical** to the pre-pin files (`cmp`) [diff]. `IS_OIDC_ENABLED` is still in the instance config
  (`license/api/views/instance.py`, `# woven: OIDC SSO` markers kept); the only change in that file is the PostHog removal (S01).
- The shared path behind them did change [diff], and the OIDC provider runs through it: (a) `complete_login_or_signup` now rejects explicitly deactivated accounts (`USER_ACCOUNT_DEACTIVATED`) and bot accounts
  (`BOT_USER_LOGIN_FORBIDDEN`, 5017) before anything else; (b) the OIDC `picture` claim becomes the avatar, and the avatar download now goes through an SSRF-safe pinned fetch, so an IdP avatar URL on an internal address will be refused and the
  code falls back to storing the raw URL (an exception returns `None`, which selects the fallback) [inferred]; (c) the three-line `# woven: shared verified-email guard` comment above `OAUTH_PROVIDER_UNVERIFIED_EMAIL` (5124) was dropped by the merge, because upstream now carries that key itself; the
  key, the `email_verified` guard in `oidc.py` and the woven `OIDC_NOT_CONFIGURED` / `OIDC_OAUTH_PROVIDER_ERROR` codes (5300, 5301) are intact; (d) the IdP-sync condition fix has no OIDC effect (`check_sync_enabled` has no OIDC key).
- Still for the dynamic half (section 11 item 10): the callback's redirects and error codes on a real round trip.

### 15.9 The 22 live-looking 404 calls

**Unchanged: the same 22 call sites, all still `no-route` on the pinned server.** `live_unmatched_added` and `live_unmatched_removed` are empty; the unmatched-by-category counts are identical (52 / 1 / 93). Two sites moved one line
(`select-repository.tsx` 37 -> 36, `ai.service.ts` 44 -> 43) and all moved paths with the layout (15.10); the group split of the 22 in section 5 stands. (The comparison tool keys sites by `Class.member` + verb, so the two
`DashboardService.getWidgetStats` sites, web and packages/services, count once: 21 keys for 22 sites in both runs.) Whether they are reachable in the pinned build is still [unverified] and remains item 2 of section 11.

### 15.10 For whoever re-runs after the next upstream sync: the layout moved

Upstream **flattened `apps/web`** between the fork base and the pin, and that is what crashed the first attempt (`shapes_probe.py` opened `apps/web/core/store/issue/helpers/issue-filter-helper.store.ts`):
- `apps/web/core/<x>` and `apps/web/ce/<x>` -> `apps/web/<x>`: **27 of the scripts' hand-curated paths moved** (components, layouts, lib/wrappers, services, store; the full list is `layout-report.json` `paths_moved_vs_hand_curated_layout`).
  `packages/editor/src/core/` and `/ce/` were flattened into `packages/editor/src/` the same way (and `src/ee/` is gone); `MOVE_RULES` covers it, but none of the 27 moved hand-curated paths is an editor path on this pin.
  `apps/api` did not move.
- **`packages/ui` and `packages/propel` no longer exist.** The web app now imports `@makeplane/propel/...` (an external package) and `@plane/blocks/...` (new in-repo `packages/blocks`, 278 files, **0** `/api` or `/auth` literals);
  `packages/i18n` shrank from 89 to 17 files. None of them holds API calls, so the client inventory is unaffected, which is why this accounts for most of the 3,283 -> 2,595 files. It is not neutral for Q-R2 ("branding", "service layer"):
  anyone planning edits in `packages/ui` / `propel` will not find them [inferred; outside this spike].

How the scripts adapt (`layout.py`): `MOVE_RULES` maps old prefixes to new; every hard-coded path goes through `Layout.resolve()`, which tries the canonical spelling and then each rule in both directions and, if nothing exists,
raises `LayoutError` **naming the path and every variant it tried**. It records `paths_moved_vs_hand_curated_layout` and `optional_paths_absent` (`packages/ui`, `packages/propel` and one README were absent on the pin: recorded, never silently skipped).
Beyond path lookup: the client scan reports directories that are neither scanned nor excluded with their `/api`|`/auth` literal count (`unlisted_dirs_not_scanned_as_clients`: 0 hits in all, including `packages/blocks`);
`shapes_probe.py` no longer swallows `FileNotFoundError` and records each anchor's expectation (`anchor_checks_violated`: 0); `p0_slice.py` has a token per trigger so a moved line is found (`slice_triggers.drift`) and a vanished one fails the run;
the override tables are keyed by the canonical (old) spelling and compared through `layout.canon()`. `p0_slice.py` and `effort-summary.json` keep the canonical trigger text so older runs stay byte-comparable; `tables.md` and the delta show the resolved
path in the scanned tree. **After the next sync:** a `LayoutError` means add a rule to `MOVE_RULES`; `api_literal_hits > 0` on an unlisted dir means add it to `client_calls.py`; read `layout-report.json` before the numbers.

### 15.11 Does anything invalidate a prior conclusion?

**No.** The answers to Q-0010.1, Q-0009.1 and Q-0011.1 stand, and so does ADR-0034's input. Figures to carry forward with their pinned values:

| statement (where) | pre-pin | pinned |
|---|---|---|
| `/api/v1` reachable entries; grand total entries (1.1, 3) | 68 (132 rows); 386 | **73 (137 rows); 391** |
| call sites targeting the API / matched / compat rows never used (1.2, 5) | 529 / 478 / 204 | **528 / 477 / 205** (internal 172 of 441, 39.0%) |
| tiers P0 / not-used (1.3, 6) | 291 / 204 | **290 / 205** |
| `this.<verb>(` call sites; files scanned (4) | 502; 3,283 | **501; 2,595** |
| literal recall (4) | 261 / 262 | 260 / 261 (99.6%) |
| rows dispatching a Celery task, compat / P0 (3) | 98 / 73 | **95 / 72** |
| matched calls without trailing slash (5, 9) | 21 | **19** |
| authentication error codes (7.4, 9, 10) | 59 | **60** |
| minimum slice; P0 compat; every row (1.4, 8) | 38.6; 207.0; 327.6 | **39.3; 207.6; 333.5** |

Three statements to **qualify**, none reversed:
1. Section 11 item 12 and section 13 ("throttle behaviour [unverified]"): partly answered from source: on the pin the sign-in/sign-up/magic POSTs are throttled at 10/minute per IP (15.7); pre-pin they were not [inferred]. Runtime behaviour (cookie of the counter, proxy IP handling) is still unverified.
2. Section 3 "Permission model": the counts are the same shape (235 `@allow_permission` rows; class permissions 310, with `[IsAuthenticated]` rows 138 -> 130 as 8 gained a stricter class) but three permission classes changed behaviour under the same names (15.3), so the F-E00b seam must be specified from the pinned bodies, not from the table.
3. Section 7.2 (query parameters) describes `order_by` and `group_by` as free-form; on the pin they are allowlisted (15.3 and 15.7). The 25 legacy parameters are unchanged.
Also, in 8.1 the closing paragraph says S14 dispatches `workspace_seed` and `track_event`: on the pin only `workspace_seed` (still 7 slice handlers with a task).

### 15.12 Limits of this re-run

- The handler-text diff compares the handler span only: it does not see the view class's `get_queryset` or mixins (except DRF-default rows), serializer `validate` / `to_representation` bodies (only declared-field counts are compared), models, signals, Celery task bodies, settings or middleware, and it cannot tell
  a refactor from a behaviour change on its own. I read the diffs of 13 changed files (`asset/v2.py`, `workspace/base.py`, `workspace/module.py`, `module/base.py`, `issue/activity.py`, `issue/comment.py`, `project/base.py`, `cycle/issue.py`,
  `webhook/base.py`, `notification/base.py`, `search/base.py`, `user/base.py`, `license/.../instance.py`) plus the authentication package, `order_queryset.py`, `paginator.py`, `issue_filters.py` and the two permission modules: all semantic, none reformatting only
  (`module/base.py` is a `Cast(...)` type fix, probably not wire-visible). The other ~84 changed handlers are **unread** [unverified]; the 29 changed v1 rows are counted, not listed.
- Client dirs cannot be proven equal to the pin commit (15.1) [unverified].
- `compare_runs.py`'s handler-text section needs both scanned trees on disk (`tree_root` in each `RUN.json`, or `--old-root` / `--new-root`); without them it prints `SKIPPED`, which is not evidence of no change.
- Everything is static, as in sections 1-14: no HAR, no response bodies, no call frequency. The estimates are the same judgement unit costs applied to new classifications.
- The `[diff]` items in 15.3, 15.7 and 15.8 were found by hand and will not reappear in a script's output on the next re-run.
