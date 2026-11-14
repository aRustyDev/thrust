# e2e-plane

What the browser journeys need to run on the **pinned Plane**, the UI oracle (issue #45; `e2e/journeys/`, `scripts/e2e.sh plane-oracle`).

This is test tooling, not product code. No crate imports it (ADR-0006 register).

The oracle stack (`tools/oracle-v1/compose.oracle.yml`) is API only: no web, no proxy. A journey drives the stock web app in a browser,
so the Plane leg needs the app beside the API, on one origin.

| File | What it is |
|---|---|
| `build-pin-bundle.sh` | builds the pin's stock app: Thrust's `web/` with every file of the Q-R2 ledger (`web/ALLOWED-CHANGES`) restored from the pin |
| `compose.ui.yml` | a compose override: a Caddy container serving that bundle and proxying `/api/*`, `/auth/*`, `/static/*` to the API, and the API's `WEB_URL`, `APP_BASE_URL` and `CORS_ALLOWED_ORIGINS` set to the browser's origin |
| `Caddyfile` | the routing of that container |

## Why the bundle is Thrust's `web/` with the ledger reverted

`web/apps/web` and `web/packages` are byte-identical to the pin except the files in the ledger, and `scripts/check-web-diff.sh` holds CI
to that. Thrust's auth client posts JSON, which Plane does not take (it takes a form and answers 302), so a Thrust bundle cannot sign a
user in on the pin. Restoring the ledger from the pin gives the pin's app, built with the same lockfile, toolchain and environment as the
Thrust bundle, so **the only variable between the two legs of a journey is the ledger**. It follows the ledger as it grows (base URL,
collab provider, branding) and leaves the fork checkout without a `node_modules`.

Each restored file is checked three ways: the blob the fork says the pin has, the entry of `web/PIN-MANIFEST`, and the hash of the bytes
written. The input is the committed `web/` at HEAD (`git archive`), not the working tree. The bundle is built into
`target/e2e-plane/pin-web` and reused while the `web/` tree, the pin and the ledger are unchanged.

## What it needs, and what it does to the machine

- The **fork checkout** with the pin, and its `oracle/` as `ORACLE_DIR` (as for `tools/compat-record/fresh-oracle.sh`). It is **only
  read**: `git show` of the ledger's files, and the compose file. The only checkout with `oracle/` and the pin tag at the time of writing
  is a worktree of the fork under `.claude/worktrees/`, on an unpushed branch (`oracle/PIN`: `pushed=false`), so it can be pruned; this
  is why the Plane leg is local only and CI runs `worker-cf` alone.
- Docker, Node and pnpm (the versions `web/` pins), and `uv`. The first run pulls the oracle's images and builds nothing but the app.
- Ports `18080` (the API) and `18088` (the app, `http://localhost:18088`) on loopback.
- `scripts/e2e.sh plane-oracle` brings the stack up **fresh** (`down -v`, as the recorder does) and takes it down after, so it **drops
  the seeded snapshot** that `scripts/oracle-v1.sh plane` needs. Re-seed after it. `E2E_KEEP=1` leaves the stack up.
- The pin **throttles** `/auth/email-check/`, `/auth/sign-in/` and `/auth/sign-up/` to 10 a minute per address and counts the refused
  calls (`AuthenticationThrottle`); the journeys make about 14 of them between them. `e2e/support/throttle.ts` holds those three calls
  in the browser until the last minute has room, on this leg only, so a run takes about a minute longer than the Thrust leg and one
  click can wait most of a minute (`playwright.config.ts` and `support/context.ts` allow for it with longer timeouts here). Without it
  the third journey is answered `429` on its first call (measured, G-34).

## Limits (left for issue #46)

- `/god-mode`, `/spaces` and `/live` are not served. A journey that opens the work-item editor's collab provider needs `/live`.
- Uploads from the browser go through this origin: the overlay sets `USE_MINIO=1`, so the pin signs the URL for the request's host
  (its own `storage.py`), and the Caddyfile sends `/uploads/*` to the object store, as Plane's proxy does. The oracle's own
  `.env.oracle` has `USE_MINIO=0`, which signs for `plane-minio:9000`, a name only the compose network resolves. The M1 journey needs
  this: making a project uploads its cover (S28). Thrust's own uploads go to a one-time PUT URL on the API (D-035), so the two legs
  upload by different routes and the journey sees only that both work.
- The Celery workers are stopped, as for the recordings, so `workspace_seed` never runs (D-024); Thrust does not reproduce it either.
- `scripts/e2e.sh plane-oracle` is not run in CI, so nothing here is checked by a pull request. What a pull request does check:
  `scripts/check-web-diff.sh` (the ledger), and the `worker-cf e2e` cell (the Thrust leg).
