---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0051: Authorization model — permission seam, RBAC, ReBAC, guests

## Context and Problem Statement

Plane CE roles are a fixed integer enum — Admin=20, Member=15, Guest=5, no Viewer — duplicated in ≥4 files, stored on `WorkspaceMember`/`ProjectMember` (`role`, `is_active`) **[S]**. Enforcement has two paths: DRF permission classes (pure membership `.exists()` checks) and the dominant `@allow_permission([...], level, creator, model)` decorator on 38 view files, whose `creator=True, model=Issue` branch is the only per-object escape hatch; guest visibility is queryset filtering via `ProjectMember.role` and `Project.guest_view_all_features` **[S]**. The superseded Open-EE program specified a **permission seam** (capability resolver behind the same decorator), **RBAC** (custom roles), **ReBAC** (an `AccessGrant` relation-tuple table with `viewer|editor|owner`, `filter_visible`, per-work-item share panel) and **guests** **[O]**. Now implemented natively (ADR-0004), the model must be chosen once and called from the **use-case layer**, not the router, so every driving adapter (HTTP, collab, queue consumers) is covered (ARCHITECTURE §2b).

## Decision Drivers

* Built-in roles must reproduce today's behaviour exactly (20/15/5 and workspace-admin-as-project-admin fallback **[S]**).
* Capability-based custom roles (RBAC) and object-level grants (ReBAC) must slot in without changing call sites.
* List endpoints need **query scoping** (`scopeQuery`) not just yes/no checks (perf-sensitive on large issue lists **[O]**).
* Runs on both adapter sets; no mandatory external service unless justified.
* Collab and public `spaces` need authorization outside the HTTP route.

## Considered Options

* Option 1: In-core capability RBAC + in-DB relation-tuple table for ReBAC, behind an `Authorizer` port
* Option 2: External ReBAC engine (OpenFGA / SpiceDB) as the authorization system of record
* Option 3: Embedded policy language (Cedar/OPA-WASM) evaluating policies over entity attributes
* Option 4: Port Plane's role enum + decorator as-is (fixed roles, `creator` bypass), defer RBAC/ReBAC

## Decision Outcome

**Decision (accepted, round 24): Option 1** — in-core capability RBAC behind an `Authorizer` port; P0 is a static capability map (Admin 20 / Member 15 / Guest 5, creator bypass, workspace-admin implies project-admin), custom roles P1, relation-tuple table reserved for P2 ReBAC; no authorization cache at P0; permission proof enforced by types.

(Earlier hypothesis, retained as rationale — the accepted outcome is above: Leading hypothesis: **Option 1**, confidence **medium**.)

Port sketch (Rust; illustrative, not final signatures):

```rust
/// Proof that a principal may read/write within a scope. Only the Authorizer can construct it
/// (private fields), and repositories accept nothing else, so a use-case cannot query without authorization.
pub struct Scope { /* workspace_id, project visibility predicate, guest restrictions */ }

pub trait Authorizer {
    /// yes/no + reason for a single action
    async fn can(&self, principal: &Principal, capability: Capability, resource: Option<&ResourceRef>)
        -> Result<Decision, AuthzError>;
    /// visibility filter for list queries; the only way to obtain a Scope
    async fn scope(&self, principal: &Principal, resource_type: ResourceType) -> Result<Scope, AuthzError>;
    /// optional: admin/audit debugging
    async fn explain(&self, principal: &Principal, capability: Capability, resource: &ResourceRef)
        -> Result<Explanation, AuthzError>;
}
```

* **Capabilities** are a typed enum with stable string names (`issue.delete`, `project.update`, …). A catalogue maps built-in roles (Admin/Member/Guest) to capability sets reproducing Plane exactly; custom roles (F-E01) are workspace-scoped sets of capabilities; members carry a role reference.
* **ReBAC** as tuples `(subject, relation, object, effect)` (subject = user/role/group; relations `viewer|editor|owner`) consulted when a resource is object-scoped; the `creator` bypass is the highest-priority implicit grant. A SpiceDB/OpenFGA adapter can implement the same port later without touching use-cases.
* **Guests**: first-class role plus `guest_view_all_features`-style project switch; comment `access` (INTERNAL/EXTERNAL) retained as the external-visibility hook **[S]**.
* Called in use-cases (and by the collab adapter on connect), never only in routes. Feature-flagged capabilities are resolved through the flags port (F-E00a).

### Consequences

* Good: no extra infra; atomic with data; parity with Plane at rollout; extension path to external engines.
* Bad: we own the resolver and tuple-store performance (indexes, N+1 avoidance in list scoping); capability catalogue must track UI feature set.
* Neutral: hierarchy (workspace admin implies project admin) encoded in the catalogue, not ad hoc.

### Confirmation

Parity test: replay a matrix of (role × endpoint) captured from live Plane against the new authorizer (ADR-0122); `scopeQuery` property tests (user never sees rows outside grants); contract tests for `Authorizer` adapters; lint: use-cases must call `Authorizer` before repository writes.

## Pros and Cons of the Options

### Option 1: In-core RBAC + tuple table

* Good: self-contained; transactional with data; maps 1:1 to Open-EE specs.
* Bad: effort to build/optimise; limited expressiveness vs Zanzibar (no deep graph rewrites).

### Option 2: External OpenFGA/SpiceDB

* Good: purpose-built, expressive, scalable; separation of concerns.
* Bad: extra service to run/secure on both adapter sets (nothing CF-native); network hop per check; list scoping requires list-objects calls or dual-write sync.

### Option 3: Embedded policy language

* Good: declarative, testable policies.
* Bad: still needs data loading for relationships; WASM size/latency on Workers; unfamiliar to maintainers.

### Option 4: Port as-is

* Good: smallest, fastest parity.
* Bad: does not satisfy superseded Open-EE requirements (custom roles, per-object access); rework later.

## Update — interview rounds 4 and 8 (2026-10-17)

The **permission seam and RBAC are P0** (round 4). The P0 role model is **Plane's fixed roles (Admin/Member/Guest at workspace and project level; levels 20/15/5), with no Viewer role** (round 8, Q-0051.1 answered). **Custom roles, ReBAC and the depth of guests are deferred to the tiering pass** (Q-F1/Q-F2). Design consequence: the `Authorizer` port and its call sites in the use-case layer must be built at P0 so custom roles and relation tuples can be added behind the same port without touching callers. The leading hypothesis (in-core capability RBAC + relation-tuple table later) is compatible; P0 can implement the fixed roles as a static capability map.

## Update — interview round 10 (2026-10-17)

Tiering: **custom roles (F-E01) P1** (corrects the earlier P0 mark in FEATURES; P0 stays fixed Admin/Member/Guest via a static capability map), **guests (F-E03) P1**, **OIDC/SAML SSO (F-E05) P1**, **ReBAC (F-E02) P2**, **SCIM (F-E04) P2**. The `Authorizer` port must still be built at P0 so these are additive.

## Update — interview round 24 (2026-10-18)

Accepted: **Option 1.** P0 is a **static capability map** (Admin 20 / Member 15 / Guest 5, creator bypass, workspace-admin implies project-admin); custom roles are P1 (capability sets in the DB); a relation-tuple table is reserved for P2 ReBAC. **No authorization cache at P0** (Q-0051.2): roles and memberships are read from the database on every request and never trusted from the session. **Revocation:** immediate over HTTP; for collab WebSockets bounded by the **5-minute signed token** with a Durable Object re-check on refresh (Q-0082.1); API keys carry the owner's current principal. **Enforcement by types:** repositories accept only a `Scope` produced by the `Authorizer`, so a use-case cannot query without authorization; a lint remains as a backstop.

## Update — review fixes, round 25 (2026-10-18)

Replaced the TypeScript `Authorizer` sketch with a Rust trait sketch including the `Scope` proof type (round 24 decision: permission proof enforced by types). Decision unchanged.

## Update — spike results, round 26 (2026-10-18)

Spike 0009 (pinned tree): three permission classes are **stricter under the same names** than in the pre-pin source, and 77 of the P0 handlers changed text — for example workspace-logo uploads now need a workspace admin, a project-bound asset needs an active `ProjectMember` of its project, and module lists are limited to projects the caller is an active member of. The F-E00b permission seam and the permission-matrix parity tests (ADR-0122) must therefore be written from the **pinned** handler bodies, not from earlier source.

## Update — owner decision, round 27 (2026-10-18)

The permission seam must enforce D-014 and D-015 (`DEVIATIONS.md`): the attachment list routes check project membership, and the `cycle-issues/` lookup is scoped to the workspace and project. Both are cross-scope defects in the pinned Plane that the owner decided to fix (Q-0033.3); the property tests for the tenant predicate (R-S1) cover them.

## Update — review round 29 (2026-10-19)

Per FR-02 and FR-03 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply now), the permission proof gates the database and the principal stops carrying a workspace. Neither changes the round-24 Decision. FR-02 makes its "permission proof enforced by types" concrete, and FR-03 supersedes the slice's `Principal` shape, which no ADR decided.

**FR-02: the proof is the argument.** Round 24 says repositories accept only a `Scope`; the slice does not. The `Database` port takes bare `WorkspaceId` and `ProjectId` (`crates/thrust-ports/src/lib.rs:92-138`), and `Scope` has private fields but public getters that hand the ids back (`crates/thrust-core/src/lib.rs:139-155`), so any caller can unwrap a `Scope` and call the port directly. Decision: repository and `Tx` methods take `&Scope`, minted only by `authorize`, instead of bare ids. Untenanted work (the outbox relay, the sweeper, the consumer and `seed`) takes a `SystemScope`, minted only by the composition roots and the consumer. `Scope`'s getters are not public outside `thrust-core`. The signatures, and the layering constraint on where the proof type is declared, are in ADR-0045 (round 29). The round-24 lint stays as a backstop.

**FR-03: the principal is the user.** `Principal { user_id, workspace_id }` (`crates/thrust-contracts/src/lib.rs:81-86`) binds one workspace to a session, and `authorize` takes the workspace from it (`crates/thrust-core/src/lib.rs:165`). Plane resolves the workspace from the URL slug on every request (`apps/api/plane/app/permissions/base.py:28,47,55`), and a user can be in several workspaces. Decision: `Principal { user_id }` only. `authorize` resolves the workspace per request from the URL slug through the membership query, and returns the `Scope`. This follows round 24's rule that roles and memberships are read from the database on every request and never trusted from the session. Sessions: ADR-0052 (round 29).

## Open Questions

* Q-0051 — model choice and seam shape.
* Q-F2 — which Open-EE authz features are P0.
* Q-0050.1 — IdP claims/groups feeding roles.
* Q-0051.1 — Is a "Viewer" role (EE) required, and how do guests differ from viewers?
* Q-0051.2 — Authorization decision caching (per request vs KV) and invalidation on role change.

## More Information

[[ADR-0004]], [[ADR-0050]], [[ADR-0053]]. Plane: `apps/api/plane/app/permissions/{base,workspace,project,page}.py`, `app/views/issue/base.py:1039-1050` (guest queryset filter, not the `creator` branch; a second `allow_permission` also exists in `utils/permissions/base.py`). Open-EE: `woven/plan:.claude/plans/plane-fork/features/{f0-permission-seam,rbac,rebac-fine-grained,guests}.md`, `research/auth-permissions.md` §2, §5.
