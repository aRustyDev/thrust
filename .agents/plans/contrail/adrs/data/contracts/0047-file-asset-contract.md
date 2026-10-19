---
status: proposed
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0047: File-asset contract

## Context and Problem Statement

Plane stores attachments, avatars, covers, and inline editor images as `FileAsset` rows pointing at S3-compatible storage (MinIO by default), with a presign → client upload → confirm lifecycle and cleanup jobs (`delete_unuploaded_file_asset` daily 02:00; `delete_old_s3_link` 01:30/03:45) **[S]**. The new system needs a storage contract that is identical over R2 (CF adapter) and S3/MinIO (self-hosted), per ADR-0002/ADR-0080, and keeps imported rows and editor-embedded URLs working. Asset visibility must follow the permission seam (ADR-0051), including public `spaces` boards.

## Decision Drivers

* Large uploads without proxying bytes through Workers where avoidable (CPU/memory/request-size limits).
* Same contract on both adapter sets (S3-compatible API is the common denominator).
* Authorization on read (private by default; signed URLs short-lived).
* Orphan cleanup and quota/size limits (`FILE_SIZE_LIMIT`).
* Compatibility with `/api/v1` assets endpoints (no imported rows — Q-G3: start fresh).

## Considered Options

* Option 1: Presigned direct-to-store upload (PUT/POST) with server-side **confirm** step; reads via short-lived signed URLs or authorized redirect
* Option 2: Worker-streamed upload and download (API proxies bytes through the binding / SDK)
* Option 3: Resumable/multipart protocol (tus or S3 multipart) as the primary path, presign per part
* Option 4: Hybrid — presigned for >N MB and browser uploads; Worker-streamed for small/server-originated files (exports, imports)

## Decision Outcome

Decision pending — see Q-0047. Leading hypothesis: **Option 4 (hybrid), with the presigned path as the contract's reference behaviour**, confidence **medium**. Both adapters expose the same `BlobStore` operations (ADR-0080) and the asset service exposes:

```
createAsset({workspace, entity_type, entity_id, name, mime, size, access}) -> { asset_id, upload: {method, url, headers, expires_at} }
confirmUpload(asset_id) -> asset            // HEAD object, verify size/mime, set is_uploaded
getAccess(asset_id, principal) -> { url, expires_at }   // permission-seam checked
delete(asset_id)                             // soft delete + deferred object delete
```

States: `pending → uploaded → (deleted)`; `pending` older than TTL is reaped by the Scheduler (replaces `delete_unuploaded_file_asset`). Object key layout `{workspace_id}/{asset_id}-{safe_name}` (no imported rows, Q-G3). Limits (`max_size`, allowed mime) are config, validated at create **and** confirm.

### Consequences

* Good: bytes bypass Workers for big files; identical flow to Plane (frontend upload code largely reusable, Q-0010); R2 presign via S3 API works.
* Bad: two code paths (presigned vs streamed); confirm step required to avoid trusting the client; CORS config on bucket; presign signing needs R2 API credentials stored as secrets.
* Neutral: image transformation (thumbnails) deferred (Q-0081).

### Confirmation

Contract test (ADR-0121) runs the lifecycle against R2 (workerd/Miniflare + a real-R2 smoke job) and a MinIO testcontainer: create→PUT→confirm→read→delete; oversize/mime-mismatch rejected at confirm; orphan reaper; unauthorized `getAccess` denied.

## Pros and Cons of the Options

### Option 1: Presigned direct

* Good: scalable, cheap, mirrors Plane.
* Bad: client trust until confirm; requires bucket CORS and credential handling.

### Option 2: Worker-streamed

* Good: simplest authorization; single path; no bucket exposure.
* Bad: Worker request/duration limits, CPU time and egress through compute; poor for large files.

### Option 3: Resumable multipart

* Good: reliability for huge files.
* Bad: protocol complexity and frontend changes; overkill for typical Plane attachments.

### Option 4: Hybrid

* Good: right tool per case.
* Bad: more code and tests.

## Update — interview round 16 (2026-10-17)

Default max asset size **5 MB per instance, configurable** (Q-0047.1). Public-board assets are served through a **Worker proxy** that serves published data only (Q-0047.2, Q-0080.1). Lifecycle contract (Q-0047) still open.

## Update — review fixes, round 25 (2026-10-18)

Defaults aligned with rounds 16 and 25: **default maximum asset size 5 MB per instance, configurable** (`max_size`; the multipart/presigned path exists for operators who raise it); **public-board assets are served through the Worker proxy** (published data only; ADR-0022, ADR-0080) — the presigned-GET path is for authenticated access only. The `createAsset`/`confirmUpload` sketch is language-neutral; its Rust form is a trait in `thrust-core` over the `BlobStore` port. The ADR stays `proposed` (lifecycle contract still open).

## Open Questions

* Q-0047 — approve contract and upload path.
* Q-0081 — uploads, image transforms, cleanup.
* Q-0010 — frontend upload service changes.
* Q-0047.1 — Maximum asset size target and whether `FILE_SIZE_LIMIT` semantics stay per-instance.
* Q-0047.2 — Public `spaces` asset access: signed URL, public bucket prefix, or Worker proxy?

## More Information

[[ADR-0002]], [[ADR-0051]], [[ADR-0080]], [[ADR-0081]]. Plane: `db/models/asset.py` (`FileAsset`), `bgtasks/file_asset_task.py`, `app/views/asset/`, `api/views/asset.py`.
