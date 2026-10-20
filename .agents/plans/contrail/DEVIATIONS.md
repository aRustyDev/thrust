# DEVIATIONS — intentional departures from Plane (exception register)

Each row is a deliberate difference from Plane CE behaviour or from the pinned oracle. A deviation needs an id, a reason, an
owner decision and a test that pins the new behaviour. Normalisation rules for the oracle diff (uuid, `sequence_id`,
timestamps, ordering; Q-0122.2) live with spike 0008.

| ID | Deviation | Plane behaviour | Decision | Source |
|---|---|---|---|---|
| D-001 | Retention default 30 days; hard-delete cleanup (F-D10) ships at P0 | `HARD_DELETE_AFTER_DAYS` = 60 | owner, rounds 20/25 | Q-0043.2 |
| D-002 | Search = full-text + trigram (substring/prefix parity) | `icontains` substring only | owner, round 25 | ADR-0083 |
| D-003 | Sessions are DB-authoritative, no cache at P0; round 29: the spike's KV store is removed (FR-03) | Django sessions (Valkey-backed cache) | owner, rounds 25/29 | ADR-0052, 0082 |
| D-004 | `apps/admin` (god-mode) retired; settings folded into workspace settings + env/IaC | instance admin app | owner, rounds 7/23 | ADR-0021 |
| D-005 | PostHog product telemetry and `instance_traces` job dropped | present | owner, rounds 18/20 | ADR-0130, 0072 |
| D-006 | Duplicate `delete_old_s3_link` schedule collapsed to one | 01:30 and 03:45 | owner, round 20 | ADR-0072 |
| D-007 | Rate limits are per-location and approximate; 10 s/60 s periods only | exact, free-form `API_KEY_RATE_LIMIT` | owner, round 15 | ADR-0033 |
| D-008 | No API key scopes in v1 | none | parity (no deviation) — recorded for clarity | ADR-0033 |
| D-009 | AI features default-off, per-workspace opt-in | n/a | owner, round 19 | ADR-0084 |
| D-010 | Social OAuth (P2) and magic code (P1) are later than password; OIDC at P1 | all providers available | owner, rounds 4/12 | FEATURES |
| D-011 | Start fresh: no import of Plane data; schema frozen at the pinned version | n/a | owner, rounds 5/20 | Q-G3, ADR-0042 |
| D-012 | Page/description editing is a plain API save at P0; collab at P1 | live collaborative editing | owner, round 6 | ADR-0060, 0012 |
| D-013 | Oracle covers `/api/v1` only; compat-plane endpoints by recordings | n/a | review, round 25 | ADR-0122, spike 0009 |
| D-014 | `GET .../issue-attachments/` and `.../attachments/` enforce project membership | any valid API key reads the attachment metadata of any project (catalogue PQ-412) | owner, round 27 (Q-0033.3) | ADR-0031, 0051, 0141; oracle exception needed |
| D-015 | `POST .../cycle-issues/` scopes the existing-`CycleIssue` lookup to the workspace and project | re-points other projects' or tenants' issues into a cycle (PQ-413) | owner, round 27 (Q-0033.3) | ADR-0031, 0051, 0141; oracle exception needed |
| D-016 | Hardened sanitiser: Plane's allow-list plus value filters on `style`, `background` and `data-emoji-url` (`hardened_builder`, `crates/thrust-editor-doc/src/sanitize.rs:169-177`); neutralises all 125 executable payloads of the spike-0006 XSS corpus and changes none of 195 editor-generated documents | nh3 0.2.18 (ammonia 4.0.0) with Plane's allow-list neutralises 116 of 125; 9 survive (six `style` values, `td background`, `td style` with `url(javascript:)`, callout `data-emoji-url`) | owner, round 29 (FR-23) | ADR-0012, Q-0012.2, C-5; `research/0008` §7 |
| D-017 | `issue-embed-component` and its attributes are allowed by the sanitiser, so page HTML keeps work-item embeds | stripped: the tag is not in `CUSTOM_TAGS` (`apps/api/plane/utils/content_validator.py:72-78`); 5 of 195 editor documents lose an embed in HTML (binary and JSON keep it) | owner, round 29 (FR-23) | ADR-0012, Q-0012.2; `research/0008` §5 row 7, §7 |
| D-018 | `a rel` is preserved as the editor writes it (`noopener noreferrer nofollow`) | `rel` is not in the `a` allow-list (`content_validator.py:114`) and nh3's default `link_rel` rewrites it to `noopener noreferrer`, dropping `nofollow`; 61 of 195 editor documents change their parsed `link.rel` | owner, round 29 (FR-23) | ADR-0012, Q-0012.2; `research/0008` §5 row 8, §7 |
| D-019 | The sanitiser runs on ammonia 4.1 or later (4.2.4 measured), whose output is not byte-identical to nh3's: it escapes `<` and `>` inside attribute values and removes whole `svg`/`math` payloads; a security improvement | nh3 0.2.18 binds ammonia 4.0.0 (raw `<` and `>` in attribute values, `svg`/`math` payloads partly left); Rust ammonia 4.0.0 matches nh3 on 430 of 430 inputs, 4.2.4 on 423 of 430 (one editor document, parse-equivalent) | owner, round 29 (FR-23) | ADR-0012, Q-0012.2; `research/0008` §7, §8 |
| D-020 | Work items store one form at P0: the sanitised `description_html` and `description_stripped` (ported `strip_tags`); JSON and binary are derived lazily at P1 by `thrust-editor-doc` on the first collab open. Pages store the triple the fallback path sends, with the HTML sanitised and the binary checked to decode with `yrs`, which is stricter than Plane | work items: the web editor writes `description_html` and `description_json`, and v1 returns `description_binary` null in 64 of 64 objects; pages: `PageBinaryUpdateSerializer` base64-decodes the binary and runs only `validate_binary_data` (10 MB size cap, at least 4 bytes, no suspicious text pattern in the first 200 bytes), never a Yjs decode (`apps/api/plane/app/serializers/page.py:180-198`, `apps/api/plane/utils/content_validator.py:29-68`, tag `oracle/v1-pin-20261018`) | owner, round 29 (FR-14) | ADR-0012, 0061; Q-F5.1, Q-0061.4; issue #48 |
| D-021 | Compat stubs outside the acceptance list: a compat-plane route the React shell calls that is not in the F-X13 acceptance list (the 48-route minimum slice plus the compat rows behind the M2 daily-use screens) answers with the cheapest shape-correct stub, an empty list, a 404 or the recorded fixture. Each stub is listed under "Compat stubs (D-021)" below; a stub leaves the list when its route is built or deleted | the route runs its handler and answers with real data. Routes the pinned Plane does not have already answer 404 there (ADR-0034, round 26), so a 404 for them is parity, not a stub | owner, round 29 (FR-01) | ADR-0034, 0013; F-X13; Q-R8; issue #46 |

## Compat stubs (D-021)

The list of D-021 stubs: one entry per route, naming the route, the stub it answers (empty list, 404 or fixture) and
the screen that calls it. Issue #46 adds an entry for each stub it writes, and the M2 exit criterion is that every
compat route outside the acceptance list is a stub listed here (`reviews/2026-10-19-fable-architecture-review.md`
§3). No compat route exists yet (`crates/` has no compat-plane crate), so the list has no entries.

## Oracle quirks (round 26 candidates; decided in round 27: reproduce all)

Spike 0008 recorded Plane's behaviour in `oracle/exceptions.yaml`. **Owner decision (round 27, Q-0033.2 and Q-0122.3): reproduce all of them, including every 500; no `fix` deviation is approved for these quirks (D-014 and D-015 are separate: two security defects, Q-0033.3).** The full catalogue of Plane quirks is `research/0007-plane-quirks-catalog.md`. The rows stay as a record. C-5 was left to Q-0012.2 and is decided separately in round 29 (FR-23): fix, as D-016.

| Candidate | Plane behaviour (pinned oracle) | Register recommendation |
|---|---|---|
| C-1 | Unhandled exceptions return 500 on eight v1 operations: `per_page` of 0 (ZeroDivisionError) or below 0, invalid `fields`/`expand`/payload shapes, and `POST /workspaces/{slug}/assets/` (an `is_server` constructor bug that upstream shares). Register entries EX-001 and EX-002 are this one failure seen through two checks | `fix` in the register; **decision: reproduce** |
| C-2 | Invalid, expired or inactive API key returns 403; only a missing key returns 401 | `reproduce`; decided round 27 (Q-0033.2) |
| C-3 | Two error-body shapes (`{"detail": …}` and `{"error": …}`); 403 instead of 404 on an unknown project; state create returns 200 | `reproduce` |
| C-4 | Undeclared nullables, `fields` projections that omit required keys, bare arrays where an envelope is documented, search results in an `{"issues": [...]}` wrapper that the schema omits (EX-011, corrected in round 27) (EX-006 to EX-011) | `reproduce`; matters for the schema-diff exceptions (ADR-0032) |
| C-5 | Sanitiser hardening (spike 0006): Plane's nh3 allow-list lets 9 of 125 legacy-browser or CSS-injection payloads survive; a hardened Rust config neutralises all 125 with no change to 195 editor documents | `fix` (recommended); **decided round 29 (FR-23, Q-0012.2): fix, as deviation D-016** |
