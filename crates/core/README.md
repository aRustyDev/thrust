# crates/core/

The inside of the hexagon (ADR-0002, ADR-0009 rule 4). **Nothing in `core/` depends on a crate outside `core/`**, and
`scripts/check-deps.sh` keeps all three free of any runtime or platform crate.

| Directory | Package | What it is |
|---|---|---|
| `types/` | `thrust-contracts` | The data types every crate shares: ids, records, events, request and error bodies. No runtime, no platform code |
| `ports/` | `thrust-ports` | The port traits (driven side). Every returned future is `Send` |
| `usecases/` | `thrust-core` | Domain rules and use-cases, the Authorizer and the `Scope` proof types. Depends on `ports` and `types` only |

The dependency direction is `types`, then `ports`, then `usecases`. Which of the three a change belongs in:
`docs/src/contributing/crates.md`.
