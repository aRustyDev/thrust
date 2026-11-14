---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0101: CI supply-chain controls for the new packages and artifacts

## Context and Problem Statement

The fork already carries substantial supply-chain CI **[S]**: release-please + commitlint (Conventional Commits drive
version bumps), SHA-pinned actions, OpenSSF Scorecard with an enforced policy (`.github/scorecard-policy.yml`:
Token-Permissions, Branch-Protection, Code-Review, Dangerous-Workflow, Pinned-Dependencies, SAST…), CodeQL,
dependency-review, gitleaks, and `woven-build.yml` which builds six images to GHCR and **signs (cosign keyless) and
attests (SBOM + SLSA provenance) by digest, then verifies before release**, with trivy gating fixable HIGH/CRITICAL and
grype reporting. The new backend adds artifacts that are **not OCI images**: Worker bundles (deployed via Wrangler/API)
and packages (Rust crates such as `thrust-ports` and the adapters, plus the adopted npm packages). We must decide how much of the gate set carries over and how Worker
bundles are attested.

## Decision Drivers

* Preserve existing gates; don't regress Scorecard enforcement.
* Worker deploys have no registry digest; provenance must bind the uploaded version to source.
* Self-hosted images continue to flow through the existing sign/attest/verify path (admission policy on woven-o11y).
* `ALL CHECKS PASSED` only counts if the required contexts actually ran.

## Considered Options

* Option 1: Carry over all gates as-is; add a Worker-bundle attestation (hash of built bundle + SLSA provenance via `actions/attest-build-provenance`, recorded with the Worker version tag)
* Option 2: Carry over gates; treat Worker bundles as ephemeral build output (no attestation), rely on source-level controls + branch protection
* Option 3: Re-platform onto a new pipeline (e.g. reusable org workflows) and drop fork-specific workflows

## Decision Outcome

**Decision (accepted, round 16): Option 1** — carry over all gates (sha-pinning, scorecard, attestation, trivy) and add Worker-bundle attestation; deploys authenticate with a GitHub OIDC identity; Rust gates (`cargo-deny`, `cargo-vet` or `cargo audit`) are added.

(Earlier hypothesis, retained as rationale — the accepted outcome is above: Leading hypothesis: **Option 1** (confidence: medium).)

### Consequences

* Good, because uniform provenance story across images and Worker bundles; deploys can verify "this version came from workflow X at commit Y".
* Bad, because Wrangler/API upload has no native attestation verification at deploy time — the verification is a CI-side gate, not a platform admission control.
* Neutral: new packages inherit the Dependabot, pinning and license (`copyright-check`) workflows.

### Confirmation

Scorecard job keeps its enforced policy; a release workflow step verifies the attestation for the exact bundle
digest it is about to upload; the `ci-required` aggregate (ADR-0100) must include supply-chain contexts.

## Pros and Cons of the Options

### Option 1: Carry over + attest bundles

* Good, because closes the sign-without-verify gap for Workers as already done for images.
* Bad, because additional workflow complexity; verify-before-upload must be ordered correctly.

### Option 2: Carry over, no bundle attestation

* Good, because minimal work.
* Bad, because Worker artifacts become the weak link in an otherwise attested chain.

### Option 3: Re-platform

* Good, because opportunity to consolidate.
* Bad, because discards working, hard-won gates; high risk for no capability gain.

## Update — interview round 16 (2026-10-17)

Accepted. **Carry over the fork's supply-chain gates (sha-pinning, scorecard, attestation) to the new packages;** deploys authenticate to Cloudflare via a **GitHub OIDC identity** (Q-0101, Q-0101.2); Worker bundle provenance is recorded via attestation (Q-0101.1).

## Update — review fixes, round 25 (2026-10-18)

Outcome rewritten to the accepted decision; artifact wording updated from npm packages to Rust crates plus the adopted npm packages.

## Update — the deploy, round 30 (2026-11-14)

The deploy pipeline (ADR-0110, issue #49) implements one half of this decision and **deviates from the other**.

* **Worker-bundle attestation: in place.** The `build` job of `deploy.yml` attests the bundle (`actions/attest-build-provenance`,
  SLSA provenance) and the deploy verifies it before using it, with `gh attestation verify --repo`, `--signer-workflow
  <repo>/.github/workflows/deploy.yml`, `--source-ref refs/heads/main` and `--deny-self-hosted-runners`
  (`.github/scripts/verify-bundle.sh`). The previous release's bundle, whose web files the new deploy keeps, goes through the same
  check, because an artifact's name is not evidence of where it came from (a pull request from a fork can upload one under any
  name). `--source-digest` is not asked for: on a `workflow_run` the workflow's own commit is the tip of `main`, not the commit
  `ci` ran on, so the commit is bound by `BUILD_SHA` inside the attested file instead. The provenance is recorded as the
  attestation, and the version carries the commit as its tag (Q-0101.1).
* **GitHub OIDC for the deploy: not in place, by the owner's decision of 2026-11-13.** Cloudflare documents no OIDC federation
  for wrangler, only an API token and the account id. Each environment (`staging`, `production` and their `-do-lifecycle`
  twins) holds a **scoped Cloudflare API token** in its secrets; the job that holds it builds nothing, and wrangler is the only
  process that sees it. This is a long-lived credential in CI, which this ADR's decision and Q-0101.2 set out to avoid. The
  follow-up is to replace it when Cloudflare documents a federation; its scopes and how to rotate it are in
  `docs/src/architecture/deploy.md`.
* **Pinning is held by a check:** `.github/scripts/lint-workflow.sh` requires every `uses:` of `deploy.yml` and its composite
  action to be a local action or a 40-digit commit, and keeps `secrets.`, `environment:` and `id-token: write` to the jobs that
  need them.

## Open Questions

* Q-0101 — gate carry-over.
* Q-0101.1 — How is a Worker bundle's provenance recorded (version tag/message, R2 manifest, Rekor entry)? Answered: an attestation, and the commit as the version's tag (round 30).
* Q-0101.2 — Does the Cloudflare account need a deploy identity bound to GitHub OIDC (no long-lived API token in CI)? Answered yes in round 16; **not achieved in round 30**: a scoped API token stands in until Cloudflare documents a federation.

## More Information

`.github/workflows/woven-build.yml`, `scorecard-policy.yml`, `rulesets/`. Related: [[ADR-0100]], [[ADR-0110]], [[ADR-0140]].
