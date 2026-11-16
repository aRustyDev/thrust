# crates/adapters/

Implementations of the port traits in `core/ports/`. The composition roots choose among them (ADR-0007). All five
packages share the `thrust-adapters-` prefix, so `package(thrust-adapters-*)` selects the group in a nextest filterset.

| Directory | Package | Builds for | What it is |
|---|---|---|---|
| `memory/` | `thrust-adapters-memory` | wasm32 and native | Every port in memory, with no runtime dependency: the reference implementation the tests run against |
| `postgres/` | `thrust-adapters-postgres` | wasm32 and native | The Database adapter: the SQL, written once against `tokio_postgres::Client` behind a `Connector`, and the migrations |
| `oidc/` | `thrust-adapters-oidc` | wasm32 and native | The OIDC relying party, over the Fetch port |
| `cloudflare/` | `thrust-adapters-cloudflare` | wasm32 only | The Workers platform through workers-rs: clock, ids, fetch, `waitUntil`, R2, Queues, rate limiting, email, the Hyperdrive connector |
| `selfhosted/` | `thrust-adapters-selfhosted` | native only | The self-hosted set on tokio: clock, ids, fetch (reqwest), the background executor, the Postgres pool, SMTP |

The Postgres adapter tests (`selfhosted/tests/pg_adapter.rs`) need Docker; see AGENTS.md.
