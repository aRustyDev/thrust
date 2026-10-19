# DEVIATIONS — intentional departures from Plane (exception register)

Each row is a deliberate difference from Plane CE behaviour or from the pinned oracle. A deviation needs an id, a reason, an
owner decision and a test that pins the new behaviour. Normalisation rules for the oracle diff (uuid, `sequence_id`,
timestamps, ordering; Q-0122.2) live with spike 0008.

| ID | Deviation | Plane behaviour | Decision | Source |
|---|---|---|---|---|
| D-001 | Retention default 30 days; hard-delete cleanup (F-D10) ships at P0 | `HARD_DELETE_AFTER_DAYS` = 60 | owner, rounds 20/25 | Q-0043.2 |
| D-002 | Search = full-text + trigram (substring/prefix parity) | `icontains` substring only | owner, round 25 | ADR-0083 |
| D-003 | Sessions are DB-authoritative, no cache at P0 | Django sessions (Valkey-backed cache) | owner, round 25 | ADR-0052, 0082 |
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

## Oracle quirks (round 26 candidates; decided in round 27: reproduce all)

Spike 0008 recorded Plane's behaviour in `oracle/exceptions.yaml`. **Owner decision (round 27, Q-0033.2 and Q-0122.3): reproduce all of them, including every 500; no `fix` deviation is approved for these quirks (D-014 and D-015 are separate: two security defects, Q-0033.3).** The full catalogue of Plane quirks is `research/0007-plane-quirks-catalog.md`. The rows stay as a record.

| Candidate | Plane behaviour (pinned oracle) | Register recommendation |
|---|---|---|
| C-1 | Unhandled exceptions return 500 on eight v1 operations: `per_page` of 0 (ZeroDivisionError) or below 0, invalid `fields`/`expand`/payload shapes, and `POST /workspaces/{slug}/assets/` (an `is_server` constructor bug that upstream shares). Register entries EX-001 and EX-002 are this one failure seen through two checks | `fix` in the register; **decision: reproduce** |
| C-2 | Invalid, expired or inactive API key returns 403; only a missing key returns 401 | `reproduce`; open (Q-0033.2) |
| C-3 | Two error-body shapes (`{"detail": …}` and `{"error": …}`); 403 instead of 404 on an unknown project; state create returns 200 | `reproduce` |
| C-4 | Undeclared nullables, `fields` projections that omit required keys, bare arrays where an envelope is documented, search results in an `{"issues": [...]}` wrapper that the schema omits (EX-011, corrected in round 27) (EX-006 to EX-011) | `reproduce`; matters for the schema-diff exceptions (ADR-0032) |
| C-5 | Sanitiser hardening (spike 0006): Plane's nh3 allow-list lets 9 of 125 legacy-browser or CSS-injection payloads survive; a hardened Rust config neutralises all 125 with no change to 195 editor documents | `fix` (recommended); undecided, Q-0012.2 |
