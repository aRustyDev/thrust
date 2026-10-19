# Research 0005 — v1 oracle harness (spike 0008 + bead plane-b6a)

**Status:** spike result, round 26. Numbers below are the spike agent's report; the coordinator verified only what is marked
**[verified]**. Harness code, normalisation rules and results are committed on the fork branch
`feat/plane-b6a-upstream-sync-oracle` (not pushed); the agent's worktree is
`/Users/asmith/repos/woven/forks/plane/.claude/worktrees/agent-a69ad036a9c59b0a8`.
(The agent was not allowed to write this file or `oracle/FINDINGS.md`; this text is its findings as returned.)

## Bottom line
The oracle works, but as **schema + exception register + golden corpus**, not schema alone. Plane's own schema fails its own
conformance in 12 failure classes (67 failures), omits 5 routed operations by design, documents no body for asset uploads, and
does not require `id` on read models. "100% conformance" (ADR-0122) is only meaningful **relative to the committed register**.

## The pin (plane-b6a)
- True merge of `upstream/preview` `879ce8b5c6` (package.json 1.4.2) into the fork. 20 conflicts, all resolved. Fork was 55 ahead /
  220 behind; now 0 behind. **[verified]** branch is 0 behind, 59 ahead of `upstream/preview` (56 at merge time plus three later commits).
- Pin = annotated local tag `oracle/v1-pin-20261018` → `ba85d4d3d3`. **[verified]** tag exists with that message. `oracle/PIN` records
  the `apps/api` tree sha; `scripts/verify-pin.sh` passes (agent-reported). **Nothing is pushed.**
- Commits **[verified]**: `ba85d4d3d3` merge · `842d921067` harness · `bf18854fe9` admin OIDC UI port · `72b2e01497` docs.
- `pnpm-lock.yaml` regenerated (pnpm 11.10.0). Upstream deleted `requirements/*.txt`; its `pyproject.toml` pins are ahead of the
  fork's. The fork's obsolete react-router `.trivyignore` entry was removed (react-router is now 8.3.0 on React 19).
- **Merge breakage found by type-check:** the fork's admin OIDC UI did not conflict textually but no longer compiled against upstream's
  propel/blocks API. Ported in its own commit; 19/19 `check:types` and `check:lint` tasks green. Admin OIDC **rendering was not verified in a browser**.
- **Fork delta:** 78 files, +3838/−209. **OIDC survived**: `/auth/oidc/` answers 302 on the running stack, an unmounted route 404s
  (control), 50 DB-free unit tests pass. The fork adds no v1 routes; its only v1 wire change is `upload_data.method` in upload
  responses, which the schema does not describe.

## Q-0122.1 — does the schema cover v1?
**No: 130 of 135 routed operations (96.3%), and not for the reason the ADR assumed.**
- Measured by enumerating routes from Django's live URL resolver and comparing with `/api/schema/`. A naive "list view-class
  methods" gave 176 and was wrong: the URL conf narrows methods per route with `as_view(http_method_names=...)` and DRF routers add
  `.format` twins and API-root views (12 twins and 4 roots excluded). 0 schema-only operations.
- The 5 gaps: 2 PUTs and 3 `server` paths that the project's own hook filters out. **v1 estimates are not routed** (404; same upstream).
- Six operations have no documented 2xx body: user-asset upload, generic asset upload, asset get, work-item attachment upload (both
  path families), project summary.
- Schema weaker than behaviour: `State.required` is only `[color, name]`; 29 operationIds end in `_2` because `issues`/`work-items`
  and `members`/`project-members` are both routed and documented.

## Q-0122.2 — normalisation and exception register
- `oracle/NORMALISATION.md` plus `oracle/exceptions.yaml` (`format_version: 1`): gate config, `scope_exclusions`, 12 entries.
  The CHECK diff exposed two rules the first set missed (dashless `uuid4().hex` storage keys; a random default project-logo icon): now covered.
- Entries: EX-001/002 500s · EX-003/004/005 undocumented 400/404 · EX-006 undeclared nullables · EX-007 `fields` projection omitting
  required keys · EX-008 member detail lacking `member` · EX-009 bare arrays · EX-010 envelope mismatch · EX-011 search results in an
  `{"issues": [...]}` wrapper the schema omits (corrected in round 27, see Corrections below) · EX-012 error body validated against a resource schema.
- `rust_expectation`: `reproduce` for 10 entries; `fix` for EX-001/002 (**deviation candidates**; no DEVIATIONS ids allocated).
- The gate fails on an unmatched failure, a stale entry, a runtime error, or a vacuous run.

## Harness evidence
- **Pass on Plane:** gate passes with the 12-entry register: 102 operations tested (78 reached a 2xx), 590 cases, all 67 failures accepted.
- **Must-fail controls:** passthrough proxy passes. Mutating a type, an enum value, or dropping a required field each **FAILS** on the
  right operation with the expected message. **Blind spot (documented):** dropping `id`, which the schema does not require, is NOT detected.
- Pipeline `oracle/scripts/run-all.sh` ran twice from a fresh stack, all stages green (once `--refreeze`, once CHECK). CHECK `cmp`/`diff`s
  manifest, schema, corpus, coverage and conventions against the committed copies.
- The first controls run failed because the gate's own fuzz corrupted the DB for later runs; every gate/control run now starts from a
  restored snapshot (`scripts/db-state.sh`).
- **Corpus:** 167 exchanges in 10 files (statuses 200, 201, 204, 400, 401, 403, 404, 405, 409, 415, 429, 500). A second recording on a
  fresh stack is byte-identical. Not covered: cycles, modules, intake, stickies, DELETEs of seeded entities. The fuzz phase is
  deterministic for one seed; a new seed finds new instances of known classes (accepted), a new class fails the gate.
- **Schema diff:** oasdiff 1.33.0: no change on identity; detects 6 of 6 injected break kinds (23 ERR changes); does not flag an additive
  change. **Never pass `--open` to oasdiff: it uploads specs to oasdiff.com.**
- **Timings:** cold image build 58 s (api image 464 MB); cold start 87–90 s. Images kept: api 464 MB, Postgres 355 MB, RabbitMQ 281 MB,
  silo 206 MB, valkey 60 MB, oasdiff 58 MB. Volumes removed.

## Corpus quirks recorded as data (not decided)
- Only a **missing** API key is 401. Invalid, expired or inactive keys give 403 `{"detail":"Given API token is not valid"}`.
- Two error-body shapes: `{"detail":...}` (DRF) and `{"error":...}` (Plane).
- PATCH/DELETE on an unknown project gives 403, not 404. State create returns 200, not 201. A project is created with `name=123` (201).
- `per_page` above max → 400; 0 or −1 → 500. Member delete is idempotent (204).
- 429 body `{"error_code":5900,"error_message":"RATE_LIMIT_EXCEEDED"}` with `Retry-After`; `X-RateLimit-Remaining` counts 4,3,2,1.
- `GET /api/v1/users/me/` has an undeclared-null `avatar_url`.
- **Upstream bug:** `POST /workspaces/{slug}/assets/` always 500s (`api/views/asset.py` calls `S3Storage(..., is_server=True)`; the constructor
  does not accept it). Upstream has the same bug.

## Q-0032 inputs (ADR-0032 hybrid decision stands)
OpenAPI 3.0.3; one `ApiKeyAuthentication` scheme (header `X-API-Key`); 130 operations with unique snake_case verb-first operationIds (the 6
undecorated viewset operations use auto ids and a lowercase `workspaces` tag); 17 tags; 94 component schemas with `...Request` (34),
`...Response` (20), `...Enum` (11) suffixes and 21 `Paginated*Response` wrappers; documented statuses 200, 201, 204, 302, 400, 401, 403, 404, 409.
The EX-006 nullables need an oasdiff exception if the Rust schema declares them nullable.

## ADR implications (coordinator to apply)
- ADR-0122: Q-0122.1 answered "no, 130/135". Reword Confirmation to "failure set equals the register; corpus replay diff-clean".
- ADR-0031/0122: drop "estimates" from the v1 list. ADR-0032 stays hybrid.
- DEVIATIONS: candidate entries for EX-001/002 (`fix`), 401-vs-403 for invalid keys (reproduce vs fix), to be decided.

## Open problems and not done
- **Shared `.git` side effects:** remote `upstream` added and 6 rc tags fetched. The fork's local `v1.4.0`, `v1.4.1`, `v1.4.2` collide with
  *different* upstream tags of the same names, so fetch refuses to overwrite them (the pin is a tag with its own name, unaffected).
- `oracle/ci/oracle.yml` has never run in CI (draft, not installed under `.github/workflows`).
- **Deferred to wave 2:** the utoipa code-first generation check and the Rust-vs-oracle differential run.
- Re-run: `oracle/scripts/run-all.sh` (CHECK) or `--refreeze` (rewrites committed artifacts).

## Corrections (round 27, from the quirks catalogue `research/0007`)
- **EX-011:** the register says search results omit `id`. They do not (corpus `work_items.json#search`); the schema documents a flat `IssueSearch` but the body is the `{"issues": [...]}` wrapper (PQ-309).
- **EX-010:** the sticky create/list mismatch is a drf-spectacular **component-name collision** on `Sticky`; the link and activity detail mismatch is a decorator documenting a dead `pk is None` branch (PQ-307, PQ-308).
- **EX-003 and EX-004 partly measure the fixture:** the seeded projects have `cycle_view`, `module_view` and `intake_view` switched off, so every cycle or module create or PATCH and every intake POST or PATCH returns the same 400 `non_field_errors`; the success shapes of those families were never exercised (PQ-056, PQ-314).
- **EX-001 is incomplete:** other 500 sources exist — a `deleted_at` body on work-item create (PQ-404), a non-object `issue` on intake create (PQ-405), array bodies (PQ-406), and two more `is_server` sites the harness could not reach (PQ-401). "Reproduce all 500s" (round 27) covers all of them.
- **Harness artefact:** the corpus asset exchanges `workspace-asset-get-not-uploaded`, `-mark-uploaded` and `-get-uploaded` use the placeholder id `MISSING-cap.wa`, so their 404 is the URLconf's `Page not found.`; they are not evidence about the handler.
- **Pre-pin v1 error bodies** were by inference broken by a `return exc` in the pre-pin error handling (PQ-503), so observations of v1 error bodies on the pre-pin fork should not be reused.
