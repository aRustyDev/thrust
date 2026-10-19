---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0085: Integrations model — GitHub/GitLab/Gitea sync, Slack, Beads, importers

## Context and Problem Statement

CE ships only vestigial integration scaffolding: models `GithubRepository*`, `GithubIssueSync`, `GithubCommentSync`, `SlackProjectSync`, `Importer`, `Integration`, `WorkspaceIntegration` exist, but no views/URLs; GitHub/GitLab/Gitea exist as
OAuth *login* only **[S][O]**. The Open-EE program (superseded by this effort, ADR-0004) specified Slack two-way (`slack-integration`) and external sync (`external-sync`: Beads, GitHub, GitLab, Gitea) as requirements, grounded
in the same findings **[O]**. We need the integration architecture on the new backend: where inbound webhooks land, how outbound delivery rides the event pipeline, how secrets are held, and how imports run.

Key requirements carried from the Open-EE specs (`git show woven/plan:.claude/plans/plane-fork/features/external-sync.md` and `slack-integration.md`):
* **Idempotent mapping**: Plane item ↔ external item keyed on `external_source` + `external_id` (fields already exist on `Issue`/`State`); replayed webhook = no-op.
* **Echo-loop suppression**: ignore events originated by the integration's own bot user/token.
* **Never bypass the issue write path**: Open-EE flagged that Django `bulk_create` skips `Issue.save()` (advisory-lock `sequence_id`, state defaults, `completed_at` tracking). In the new backend the equivalent is "integrations call the same use-cases
  as the API" — never raw repository inserts.
* **Signature + replay verification** on inbound (Slack signing secret + timestamp; per-forge webhook signatures); SSRF-hardened outbound.
* **Secrets** (bot tokens, signing secrets) via the `Secrets` port, never in plain DB columns.
* Beads sync: no code today; cleanest through the public `/api/v1` with an API key.

## Decision Drivers

* Integrations are *consumers and producers of the event stream* (ADR-0046/0073), not special cases in request handlers.
* Same behavior on both adapter sets; inbound endpoints are plain HTTP (driving adapter).
* Tenant isolation and per-workspace enablement (feature flags; Open-EE F0.1).
* Keep the write path single (permission seam + sequencing + activity).

## Considered Options

* Option 1: **First-party integration module** in the backend: `IntegrationProvider` interface (`install`, `verifyInbound`, `handleInbound`, `deliverOutbound`, `mapIds`), providers implemented in-repo (Slack, GitHub, GitLab, Gitea, Beads), events via outbox
* Option 2: **Webhook + public API only**: ship no provider code; documented outbound webhooks and `/api/v1` let external glue (n8n, Zapier, custom Workers) do sync
* Option 3: **Separate integration services** (own Workers/containers) that talk to Plane only through `/api/v1` and webhooks, deployed independently
* Option 4: **Plugin/app platform** (installable apps with OAuth scopes, manifest) like a marketplace

## Decision Outcome

**Decision (accepted, rounds 15 and 23): no integrations at P0; GitHub and Beads sync at P1 (F-E25); last-write-wins with echo suppression; integration credentials stored encrypted per workspace.** Slack, GitLab, Gitea and importers follow the tiers in FEATURES.md. Whether each provider is a first-party module (Option 1) or a separate service over `/api/v1` and webhooks (Option 3) is **not decided here** — it is settled at P1 planning; either way integrations call the same use-cases as the API (never raw repository inserts) and consume the event pipeline (ADR-0046/0073). The earlier mixed hypothesis (Option 3 for forge/Beads, Option 1 for Slack) is withdrawn as a decision input. Importer bulk path (Q-0085.2) remains open.

### Consequences

* Good: reusing the event pipeline means a new integration is another consumer; mapping tables are additive.
* Good: separate services (Option 3) limit blast radius of third-party API churn.
* Bad: Option 3 requires API completeness (everything an integration needs must be in `/api/v1`), increasing v1 surface.
* Bad: echo-loop and ordering problems are inherent to bidirectional sync regardless of option; need explicit design (Q-0085.3).

### Confirmation

* Round-trip idempotency tests per provider (create/edit on both sides; replayed webhook produces no duplicate).
* Signature tests: invalid Slack/forge signature → 401; stale timestamp → rejected.
* Write-path test: every integration inbound path increments `sequence_id` via the standard use-case (no gaps/duplicates under concurrency).
* Secrets test: no token material in logs or DB dumps.

## Pros and Cons of the Options

### Option 1: First-party module

* Good: deepest integration, one deployment, shared authz/events. * Bad: grows the core; third-party SDK churn in the main bundle (Workers size limit 64 MiB, plenty but still coupling).

### Option 2: Webhooks + API only

* Good: smallest scope. * Bad: no first-class UX (install, repo mapping UI); doesn't meet Open-EE parity targets.

### Option 3: Separate services

* Good: isolation; independent release cadence; dogfoods v1. * Bad: more deployables on both adapter sets; secrets/config duplication.

### Option 4: App platform

* Good: extensible long-term. * Bad: large product surface (manifests, scopes, review); premature.

## Update — interview round 15 (2026-10-17)

No integrations at P0; GitHub/Beads sync is P1 (F-E25). Remaining provider tiering follows FEATURES.md (Q-0085.1 answered).

## Update — interview round 23 (2026-10-18)

Accepted. **GitHub and Beads sync at P1; last-write-wins with echo suppression; integration credentials stored encrypted per workspace** (Q-0085.3, Q-0085.4). Importer bulk path (Q-0085.2) remains open.

## Update — review fixes, round 25 (2026-10-18)

Decision line rewritten to the accepted P1 GitHub+Beads outcome; the contradicting option hypothesis is withdrawn as a decision input and answered questions are marked. Substance unchanged.

## Open Questions

* Q-0085 — integration tiering and policy. **Answered (rounds 15, 23);** module-vs-service architecture deferred to P1 planning.
* Q-0085.1 — which providers are P0/P1. **Answered (round 15): none at P0; GitHub/Beads P1.**
* Q-0085.2 — importers (`Importer`, Jira/GitHub services): bulk path must reproduce `sequence_id` allocation — through use-cases only, or a dedicated bulk-import use-case with an explicit sequence-block reservation?
* Q-0085.3 — **Answered (round 23): last-write-wins with echo suppression.**
* Q-0085.4 — **Answered (round 23): encrypted per workspace** (`Secrets` port design to confirm).

## More Information

* Plane: `db/models/integration/{github,slack,base}.py`, `db/models/importer.py`; `authentication/provider/oauth/{github,gitlab,gitea}.py`; `bgtasks/webhook_task.py`.
* Open-EE (read-only, branch `woven/plan`): `.claude/plans/plane-fork/features/external-sync.md`, `slack-integration.md`.
* Related: [[ADR-0004]], [[ADR-0031]], [[ADR-0046]], [[ADR-0051]], [[ADR-0071]], [[ADR-0073]], [[ADR-0092]].
