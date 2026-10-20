---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0080: `BlobStore` port — R2 adapter and S3/MinIO adapter

## Context and Problem Statement

Plane stores attachments, avatars, covers, inline images and export files in S3-compatible storage (MinIO by default; boto3 +
`django-storages`) **[S]**. Assets are tracked by `FileAsset`; `bgtasks/file_asset_task`, `copy_s3_object`, `storage_metadata_task`,
`exporter_expired_task` manage lifecycle **[S]**. The interview ([I] round 3) decided R2 for attachments/exports with a self-hosted swap
(S3/MinIO) behind an interface.

## Decision Drivers

* One port, two adapters; R2 is S3-API-compatible so the S3 adapter can serve both in some modes.
* Browser uploads should not stream through the API (direct upload via presigned URL), but small server-side writes must work.
* Presigned URLs must be generated server-side without extra round trips.
* Lifecycle rules (expiry of exports, incomplete uploads).

## Considered Options

* Option 1: `BlobStore` port with an R2-binding adapter (Workers) and an S3 adapter (MinIO/S3), presign via S3 SigV4 in both
* Option 2: Single S3-protocol adapter used for both (R2 via its S3 endpoint), no Workers R2 binding
* Option 3: Port wraps a higher-level "asset service" (upload sessions, metadata) rather than bytes
* Option 4: Keep MinIO/S3 everywhere; no R2

## Decision Outcome

Chosen option: **Option 1**, because it gives native R2 bindings (no credentials in the Worker for server-side read/write) while keeping an S3 adapter
for self-hosting; presigned URLs use SigV4 in both. Source: interview round 3. Asset *lifecycle* (the `FileAsset` state machine) is a core/contract
concern (ADR-0047/0081), not part of this byte-level port.

```rust
// crate: thrust-ports (round 25: Rust sketch; replaces the earlier TypeScript sketch)
#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, key: &BlobKey, body: BlobBody, meta: PutMeta) -> Result<BlobInfo, BlobError>;      // BlobBody: bytes or a byte stream
    async fn get(&self, key: &BlobKey, range: Option<ByteRange>) -> Result<Option<BlobObject>, BlobError>;
    async fn head(&self, key: &BlobKey) -> Result<Option<BlobInfo>, BlobError>;
    async fn delete(&self, keys: &[BlobKey]) -> Result<(), BlobError>;
    async fn list(&self, prefix: &str, cursor: Option<Cursor>) -> Result<Page<BlobInfo>, BlobError>;
    async fn copy(&self, src: &BlobKey, dst: &BlobKey) -> Result<(), BlobError>;
    async fn presign(&self, op: PresignOp, key: &BlobKey, opts: PresignOpts) -> Result<Url, BlobError>;   // SigV4; works in wasm32
    async fn create_multipart(&self, key: &BlobKey, meta: PutMeta) -> Result<Box<dyn MultipartUpload>, BlobError>;
}
```

### Consequences

* Good, because R2 limits cover Plane's needs: single PUT up to 5 GiB (4.995 GiB), multipart up to 5 TiB (parts 5 MiB–5 GiB, ≤ 10,000 parts), presigned URL expiry 1 s–7 days
  (R2 docs). Plane's `FILE_SIZE_LIMIT` is far below these.
* Good, because incomplete multipart uploads auto-abort after 7 days by default (lifecycle-configurable) — natural cleanup.
* Bad, because **presigned URLs work only on the S3 API domain, not custom domains; POST form uploads aren't supported** (R2 docs) — browser upload must use PUT, and
  download URLs through a custom domain need the Worker or WAF HMAC validation.
* Bad, because a Worker's inbound request body is bounded by Workers request limits; large uploads must go direct-to-R2 (ADR-0081).
* Bad, because concurrent writes to the same key at high rate return HTTP 429 on R2 — avoid single hot keys (collab snapshots, ADR-0061).

### Confirmation

* Contract suite (ADR-0121): put/get/head/delete/list/copy/range/multipart/presign round-trips, against R2 (workerd/Miniflare R2 from the wasm32 test leg + a real-R2 smoke job) and a MinIO testcontainer.
* Lint: no S3/R2 client imports outside adapter crates (workspace dependency check).

## Pros and Cons of the Options

### Option 1: R2 binding + S3 adapter

* Good, because native and credential-free on CF; self-hosted path unchanged.
* Bad, because two code paths for put/get (bindings vs S3 SDK); presign needs R2 access keys as secrets even on CF.

### Option 2: S3 protocol only

* Good, because one adapter. * Bad, because needs R2 API credentials for every operation and extra network hop semantics vs binding; slower in-Worker path.

### Option 3: Asset-service port

* Good, because hides presign/multipart orchestration. * Bad, because mixes lifecycle policy into a storage port; harder to contract-test.

### Option 4: MinIO/S3 only

* Good, because zero change. * Bad, because not Cloudflare-native; violates interview decision.

## Update — review fixes, round 25 (2026-10-18)

TypeScript sketch replaced by a Rust trait (ADR-0006). **Public-read assets (avatars, covers, public-space images) are served through a Worker proxy that enforces "published data only"** (round 16, Q-0080.1; ADR-0022, ADR-0047), not a public R2 bucket or custom domain. The default per-instance asset size limit is **5 MB**, configurable (round 16, Q-0047.1). R2 binding and SigV4 presign from Rust/wasm32 are unverified and covered by spike 0001.

## Update — spike results, round 26 (2026-10-18)

The `?Send` sketch above is corrected (ADR-0002). Spike 0001: R2 put, get and delete are native and `[tested]` locally through the attachment tests (`list` compiled, not exercised). **The Worker root buffers request bodies, drained with an 8 MiB bound**, so a streaming `BlobBody` upload through the Worker is not available; larger uploads use presigned PUT (ADR-0081). Reason: `worker::Body` holds a reader on the JS request stream and cancels it on drop; after an early error response workerd rejects the cancel, which reset other in-flight connections in `wrangler dev` (2 of 8 runs failed, 0 of 16 after draining). Whether production workerd tolerates the unfixed shape is unverified.

## Update — review round 29 (2026-10-19)

Per FR-19 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply now, local-disk blob store at P0, S3 adapter at P1), the self-hosted `BlobStore` adapter at P0 is a local-disk adapter inside the single native binary (ADR-0091, round 29). The S3-compatible adapter (MinIO or S3) arrives at P1. The Cloudflare adapter stays the R2 binding. Reason: each self-hosted service costs a compose service, a testcontainer and a CI cell (FR-05), and FR-19 takes the MinIO service out of P0.

Today the native server keeps blobs in process memory (`MemoryBlobStore`, `crates/thrust-server/src/main.rs:83`), so a restart loses attachments (`docs/src/architecture/infra.md:91-96`); the local-disk adapter is not built yet. The built trait (`crates/thrust-ports/src/lib.rs:199-205`) has `put`, `get`, `delete` and `list`, which local disk covers directly. The sketch's `presign` has no SigV4 endpoint on local disk; how the self-hosted set answers presigned uploads before the P1 S3-compatible adapter lands is open under Q-0081.

This supersedes the Decision's S3/MinIO adapter as the P0 self-hosted adapter (it becomes the P1 adapter) and the Confirmation's MinIO testcontainer at P0: until P1 the self-hosted leg of the contract suite is the `native-pg` cell, which runs the local-disk adapter once it is built.

## Open Questions

* Q-0081 — upload flow, image transforms, cleanup of unuploaded assets.
* Q-0080.1 — **answered (round 16)**: Worker proxy, published data only.
* Q-0080.2 — key layout and tenant prefixing (workspace-scoped prefixes for isolation/export/delete per ADR-0041).

## More Information

* Plane: `FileAsset` model, `bgtasks/file_asset_task.py`, `copy_s3_object.py`, `storage_metadata_task.py`; compose service `plane-minio`.
* CF: R2 limits <https://developers.cloudflare.com/r2/platform/limits/>, upload methods <https://developers.cloudflare.com/r2/objects/upload-objects/>, presigned URLs <https://developers.cloudflare.com/r2/api/s3/presigned-urls/>.
* Related: [[ADR-0002]], [[ADR-0041]], [[ADR-0047]], [[ADR-0081]], [[ADR-0121]].
