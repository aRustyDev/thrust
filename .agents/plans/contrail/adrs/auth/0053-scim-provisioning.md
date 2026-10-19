---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0053: SCIM / user and group provisioning

## Context and Problem Statement

SCIM is entirely absent from Plane CE; provisioning is invite-based (`WorkspaceMemberInvite`, `ProjectMemberInvite`) plus JIT signup in `Adapter.complete_login_or_signup()` **[S]**. There is also **no Group primitive** to map SCIM Groups onto **[S][O]**. The superseded Open-EE spec proposed a `scim` app with `ScimToken`, `Group`/`GroupMember`, `/scim/v2/{Users,Groups,ServiceProviderConfig,Schemas,ResourceTypes}`, `externalId` mapping, deactivation instead of hard delete, and a settings UI for token issuance **[O]**. With Zitadel as IdP (ADR-0050) we must decide whether and how to provision users/groups.

## Decision Drivers

* Zitadel capability for SCIM/actions and the real need for automated deprovisioning.
* Deprovisioning must deactivate (preserve `created_by` and audit chains), revoke sessions/keys (ADR-0052).
* A Group/Team primitive is needed anyway for RBAC, ReBAC subjects and Team Spaces (ADR-0051, F-E15).
* Effort: SCIM conformance is fiddly (filters, PATCH `Operations`, pagination); IdP quirks.

## Considered Options

* Option 1: Native SCIM 2.0 server in the API Worker (Users + Groups) with a per-workspace bearer token
* Option 2: No SCIM now — OIDC JIT provisioning + invites; periodic reconcile job pulling from the IdP's management API
* Option 3: Delegate to IdP-side automation (Zitadel Actions/webhooks calling our internal provisioning API)
* Option 4: Native SCIM Users only (no Groups) first; Groups later

## Decision Outcome

**Decision (accepted, round 23): SCIM is P2 and only if the Zitadel deployment supports outbound SCIM (Q-0053.1 is the gating check); JIT provisioning via OIDC claims at P1 (Q-0050.1); the Group primitive is designed now so SCIM can follow.**

(Earlier hypothesis, retained as rationale — the accepted outcome is above: Leading hypothesis: **Option 2 for the first release, with the Group primitive designed now so Option 1/4 can follow**, confidence **low**, because the environment is single-org on Zitadel, JIT + group claims (Q-0050.1) cover most needs, and SCIM is Enterprise-tier work whose priority is not yet set (FEATURES F-E04, Q-F1/Q-F2). A decision flip to Option 1 is cheap if the `Provisioning` use-cases (create/deactivate member, set group membership) are written as ports-independent services now — SCIM then becomes a thin driving adapter.)

### Consequences

* Good (Opt 2 now): no SCIM conformance burden early; deprovisioning via reconcile; shared provisioning use-cases reusable by SCIM later.
* Bad: deprovisioning lag until reconcile runs; no standard IdP-push for other IdPs.
* Neutral: `externalId` stored on User/Group from day one (mirrors the `external_id` pattern **[S]**) so SCIM can adopt data without migration.

### Confirmation

Provisioning use-case tests (create → member present; deactivate → sessions/keys revoked, rows soft-retained). If Option 1 is later chosen: RFC 7644 filter conformance tests (`userName eq "x"`), PATCH operations, pagination, against Zitadel and a reference client.

## Pros and Cons of the Options

### Option 1: Native SCIM server
* Good: standard; push-based; works with any IdP. Bad: spec conformance cost; token management UI; Group model required first.
### Option 2: JIT + reconcile
* Good: minimal now; fits single IdP. Bad: not standard; lag; custom code against Zitadel API.
### Option 3: IdP Actions → internal API
* Good: near-real-time without SCIM spec. Bad: Zitadel-specific; couples to IdP features; auth for internal API.
### Option 4: Users-only SCIM
* Good: covers the main deprovisioning need. Bad: no group sync; still conformance work.

## Update — interview round 23 (2026-10-18)

Accepted. **SCIM is P2 and only if the Zitadel deployment supports outbound SCIM** (Q-0053.1 is the gating check); **JIT provisioning via OIDC claims at P1** (Q-0050.1).

## Update — review fixes, round 25 (2026-10-18)

Outcome rewritten to state the accepted decision; the earlier Option-2 hypothesis is kept as rationale.

## Open Questions

* Q-0053 — SCIM scope and tier.
* Q-0050.1 — claims/group mapping; JIT vs SCIM.
* Q-F2 — Open-EE feature priorities.
* Q-0053.1 — Does Zitadel (this deployment) support outbound SCIM or only Actions/management API? Verify before choosing Option 1 vs 3.

## More Information

[[ADR-0004]], [[ADR-0050]], [[ADR-0051]], [[ADR-0052]]. Open-EE: `woven/plan:.claude/plans/plane-fork/features/scim.md`, `research/auth-permissions.md` §5. Plane: `db/models/workspace.py` (`WorkspaceMemberInvite`), `authentication/adapter/base.py`.
