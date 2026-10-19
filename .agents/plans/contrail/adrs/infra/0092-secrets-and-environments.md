---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0092: Secrets management and environment promotion

## Context and Problem Statement

The system needs secrets (OIDC client secret, DB/Hyperdrive credentials, R2 keys for the self-hosted S3 path, email,
webhook-signing, API-key pepper, optional OpenAI) and a dev→staging→prod promotion model on both adapter sets. Plane
today reads everything from env vars (`.env.example`, `POSTGRES_*`, `AWS_*`, `OPENAI_API_KEY`, …) **[S]**. Verified CF
facts: Worker secrets are environment variables hidden after set; they are **per-environment and non-inheritable**
in Wrangler environments; a `secrets.required` config property validates presence at `wrangler deploy`/`versions upload`
and drives type generation; **Secrets Store** is an account-level store (beta) bound via `secrets_store_secrets`;
`vars` must never hold secrets; `.dev.vars`/`.env` for local only.

## Decision Drivers

* Plaintext must never be committed or echoed; developers read from 1Password via `op read` (per user rules) and inject into the environment, not argv.
* A single typed `AppConfig` (ADR-0007) must describe required secrets for either adapter set.
* Deploys must fail fast on missing secrets.
* Rotation without code change.

## Considered Options

* Option 1: Per-Worker Wrangler secrets (`wrangler secret put` / `versions secret put`), declared via `secrets.required`; self-hosted uses env/files; 1Password as source of truth fed by CI
* Option 2: Cloudflare Secrets Store (account-level bindings) for shared secrets, Wrangler secrets for per-Worker ones; self-hosted uses Vault/K8s Secrets
* Option 3: External secret manager (1Password Connect / Vault / AWS SM) fetched at runtime by the app through a `Secrets` port

## Decision Outcome

**Decision (accepted, round 16): Option 1** — per-Worker Wrangler secrets with `secrets.required` as the declared contract; 1Password locally and GitHub environment secrets in CI; a `Secrets` port abstracts access; revisit Secrets Store when it leaves beta. **OpenTofu manages Cloudflare *resources* only — secret values never enter OpenTofu state.**

(Earlier hypothesis, retained as rationale — the accepted outcome is above: Leading hypothesis: **Option 1** (confidence: medium) with `secrets.required` as the
declared contract and 1Password→CI injection; revisit Secrets Store when it leaves beta. A `Secrets` port still
abstracts *access* so Option 3 can be added for the self-hosted twin.)

### Consequences

* Good, because deploy-time validation catches missing secrets; no runtime dependency on a third party.
* Bad, because per-environment, per-Worker duplication (a secret needed by `api` and `consumer` is set twice) — Secrets Store would remove this.
* Neutral: rotation = new Worker version via `versions secret put` then gradual deploy (ADR-0110).

### Confirmation

CI step lints that `wrangler` config has no secret-looking `vars`, `secrets.required` matches the `AppConfig` schema, and
gitleaks (already in fork CI) passes. A staging deploy must fail when a required secret is removed (negative test).

## Pros and Cons of the Options

### Option 1: Wrangler secrets + `secrets.required`

* Good, because native, validated at deploy, works with gradual deployments (`wrangler versions secret …`).
* Bad, because duplication across Workers and environments.

### Option 2: Secrets Store + Wrangler secrets

* Good, because shared account-level secrets and RBAC (Secrets Store Admin role).
* Bad, because beta; extra access-control surface; no self-hosted equivalent.

### Option 3: Runtime secret manager

* Good, because one mechanism for both adapter sets.
* Bad, because adds a runtime dependency and latency; secrets cached in isolate memory; outage couples to availability.

## Update — interview round 15 (2026-10-17)

Direction (round 15): 1Password locally, GitHub environment secrets in CI, Cloudflare-side secrets provisioned by OpenTofu. Environments and PR-preview isolation (Q-0092.2) remain open, so this stays `proposed`.

## Update — interview round 16 (2026-10-17)

Accepted. Environments: **dev (PR previews on a shared dev DB, per-PR schema/prefix), staging, prod** (Q-0092.2). Secrets: 1Password locally, GitHub environment secrets in CI, Cloudflare-side secrets provisioned by OpenTofu.

## Update — review fixes, round 25 (2026-10-18)

Reconciled the round-15/16 wording 'Cloudflare-side secrets provisioned by OpenTofu': that is **superseded** by Option 1. Secret *values* are set with `wrangler secret put` / `versions secret put` (or Secrets Store when GA) from CI, sourced from GitHub environment secrets (1Password locally); OpenTofu creates the resources (Workers, bindings, stores) but never holds secret values, which would otherwise land in tofu state. Environments (dev, staging, prod) are unchanged.

## Open Questions

* Q-0092 — secrets mechanism.
* Q-0092.1 — Which secret source feeds CI: 1Password service account vs GitHub environment secrets? (Avoid argv exposure; `op read` inside one shell step.)
* Q-0092.2 — Environment set: dev / preview-per-PR / staging / prod; do PR previews get isolated DBs and secrets?

## More Information

CF docs: Workers secrets, Wrangler environments, `secrets` config property, Secrets Store integrations.
Related: [[ADR-0007]], [[ADR-0050]], [[ADR-0093]], [[ADR-0110]].
