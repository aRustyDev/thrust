# crates/api/

The HTTP surfaces. Each is a route group over the use-cases in `core/`, composed into one router that both composition
roots compile in.

| Directory | Package | What it is |
|---|---|---|
| `core/` | `thrust-api` | The shared axum router, compiled into both `server/` and `worker/` |
| `v1/` | `thrust-api-v1` | Plane's public API (`/api/v1`), wire-compatible (ADR-0031, ADR-0034) |
| `compat-plane/` | `thrust-compat-plane` | Plane's internal API (`/api/`, `/auth/`) for the adopted React app; transitional (ADR-0034) |
| `plane-types/` | `thrust-plane-types` | What the two Plane-shaped surfaces share: the pinned Plane's 500 bodies and the function that answers them (#51) |

Plane-shaped types stay in the crate that serves them (#52). `thrust-api` takes `api/core/` because its manifest names
it ARCHITECTURE §2a's `thrust-api-core`; if the core REST surface planned for M4 becomes a crate of its own, it needs
another directory, or `thrust-api` moves (ADR-0009, Consequences).
