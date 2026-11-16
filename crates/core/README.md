# crates/core/

The inside of the hexagon (ADR-0002, ADR-0009 rule 4). **Nothing in `core/` has a normal or build dependency on a crate
outside `core/`** (`scripts/check-layout.sh` checks it), and `scripts/check-deps.sh` keeps all three free of any runtime or
platform crate. A dev-dependency is allowed: `thrust-core`'s tests run against `thrust-adapters-memory`, which reaches
neither core's build nor what it ships.

| Directory | Package | What it is |
|---|---|---|
| `types/` | `thrust-contracts` | The data types every crate shares: ids, records, events, request and error bodies. No runtime, no platform code |
| `ports/` | `thrust-ports` | The port traits (driven side). Every returned future is `Send` |
| `usecases/` | `thrust-core` | Domain rules and use-cases, the Authorizer and the `Scope` proof types. Depends on `ports` and `types` only |

The dependency direction is `types`, then `ports`, then `usecases`. Which of the three a change belongs in:
`docs/src/contributing/crates.md`.
