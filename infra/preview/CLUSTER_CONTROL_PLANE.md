# Snapshot plus per-PR database clusters

The Preview control plane reuses an **immutable base snapshot**, never a live database or
Postgres cluster. A base snapshot is keyed by the exact trusted migration-artifact SHA-256,
Postgres major version, and bootstrap SHA-256. Two PRs whose inputs produce the same fingerprint
may use the same base snapshot, but they receive different live clusters, PVCs, database
credentials, VPC services, Hyperdrive configurations, and Cloudflare bindings.

The `preview_cluster_plan` function in `thrust-preview-provisioner` is the canonical naming
contract. For PR `n` it produces:

| Value | Identity | Reuse rule |
|---|---|---|
| `base_snapshot` | fingerprint prefix | Reuse only when every fingerprint input matches. It is sealed after creation. |
| `cluster` | PR number + fingerprint prefix | One active mutable cluster per PR and fingerprint. Never share between PRs. |
| `idempotency_key` | `preview/<pr>/<full fingerprint>` | Replay creates or returns only that PR's desired cluster. |

The fingerprint input is a trusted artifact digest, not a schema name. A schema name says
nothing about its migrations, PostgreSQL major version, bootstrap extensions, or bootstrap grants.
The migration-source authorization flow must produce the artifact before this request is made.

`PreviewClusterLease` and `reconcile_cluster_lease` are the first controller implementation.
They select one idempotent action per retry and reject partial resource inventories rather than
trying to guess how to repair them. Creation is snapshot -> cluster -> connectivity -> durable
ready receipt; PR deletion reverses only the mutable resources and leaves the shared snapshot to
the snapshot garbage collector. A changed fingerprint becomes a separate provisioning candidate,
so the old ready lease stays attached until the caller has verified the candidate receipt.

The lease is serialized only as the bounded `thrust.dev/preview-cluster-lease/v1` document. On
decode, the controller recomputes its fingerprint, snapshot name, cluster name and idempotency
key from the artifact/bootstrap digests and PostgreSQL major; it rejects any mismatch or unknown
field. The document carries no secret and has no quota/profile field. Authentication, controller
profile selection and receipt persistence remain server-side responsibilities.

A `thrust.dev/preview-cluster-receipt/v1` document is accepted only for a ready lease. It repeats
the validated lease, its derived VPC-service and Hyperdrive configuration names, and supplies
only the CNPG cluster UID, VPC service ID and Hyperdrive ID. Those IDs are opaque identifiers,
not credentials. Before emitting a receipt, the controller must verify each ID against the live
resource and the lease; before deployment, the binding path must verify the receipt again.

## Controller boundary

GitHub Actions remains without Kubernetes credentials. The Access-protected Cloudflare provisioner
will call a new internal, Kubernetes-side preview-cluster controller. That controller alone gets a
namespace-scoped Kubernetes service account. It must:

1. Persist a lease before creating resources, and treat a matching `idempotency_key` as a read or
   retry. A different fingerprint for an active PR is a staged replacement, not permission to
   attach another PR to its existing database.
2. Build a base database from the verified artifact once, seal it, and create a CNPG recovery
   source/volume snapshot only after migration validation and smoke checks pass.
3. Create one CNPG `Cluster` for the PR from that base. Give its Postgres Pod CPU/memory limits,
   `storage` and `walStorage` PVC sizes, and a cluster-local connection ceiling from a protected
   controller profile. The request API has no fields that select a profile, quota, storage class,
   image, namespace, or credential.
4. Create unique credentials in a Kubernetes Secret and a unique `*-rw` CNPG service target.
   Never expose a database password in the Cloudflare Worker response, Action logs, a ConfigMap,
   or a PR artifact.
5. Ask the Cloudflare resource controller to create a VPC TCP service pinned to that one `-rw`
   hostname and port 5432, then create a unique Hyperdrive configuration with caching disabled.
   A VPC service is an exact host/port boundary, not a tunnel-wide network grant.
   `infra/cloudflare/hyperdrive.sh ensure-preview-cluster` is the bounded Hyperdrive operation:
   it accepts only the canonical cluster name, a supplied opaque dedicated VPC-service ID and
   `pr_<n>` matching that name. Its matching `verify-preview-cluster` operation returns only the
   verified Hyperdrive/VPC IDs, never the origin password. The resource controller must not use
   the legacy `ensure` command or `thrust-pg-dev` for this path.
6. Return a non-secret receipt only after the cluster, VPC service, and Hyperdrive origin all
   match the lease. The deployment path verifies the receipt before binding the Preview.

The controller profile is the enforcement point for per-preview capacity. Kubernetes
`ResourceQuota` limits a namespace aggregate; it does not cap a Postgres schema. The controller
therefore sets hard per-cluster PVC capacities and Pod resource limits, while a namespace
`ResourceQuota` caps aggregate preview PVC count/capacity and CPU/memory. Storage class support
for `VolumeSnapshot`/clone and the hard-cap behaviour of requested PVC size must be verified
against the live CSI driver before this path is enabled.

## Lifecycle and safety

The cluster controller owns a dedicated preview namespace; its Kubernetes service account has no
rights outside that namespace. Kubernetes RBAC cannot make label selectors a write authorization
boundary, so requests must be name-validated against this contract and lease state before every
write. The Cloudflare Worker has no Kubernetes credential.

For a new head that changes the fingerprint, create and verify the replacement cluster first.
Only then rotate the unique Hyperdrive/configuration binding and remove the old cluster. A failed
or interrupted replacement leaves the last known-good cluster attached. On PR closure, unbind
the deployed Worker, delete Hyperdrive and VPC service, delete the live cluster/PVC/Secret, and
retain or garbage-collect the base snapshot according to an operator-owned retention policy.
Never garbage-collect a snapshot with an active lease.

The existing `pr_<n>` schema provisioner and `thrust-pg-dev` reconciler are a legacy path. They
remain in place until a live controller, snapshot-capable storage, dynamic VPC service lifecycle,
and migration/deployment receipt verification have passed the activation runbook. Do not enable
the migration executor based on this document alone.
