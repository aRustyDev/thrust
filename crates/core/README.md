# crates/core/

The inside of the hexagon (ADR-0002, ADR-0009 rule 4). **Nothing in `core/` has a normal or build dependency on a crate
outside `core/`** (`scripts/check-layout.sh` checks it), and `scripts/check-deps.sh` keeps all four free of any runtime or
platform crate. A dev-dependency is allowed: `thrust-core`'s tests run against `thrust-adapters-memory`, which reaches
neither core's build nor what it ships.

| Directory | Package | What it is |
|---|---|---|
| `types/` | `thrust-contracts` | The data types every crate shares: ids, records, events, request and error bodies. No runtime, no platform code |
| `ports/` | `thrust-ports` | The port traits (driven side). Every returned future is `Send` |
| `html/` | `thrust-html` | The HTML sanitiser (ammonia and Plane's allow-list) and the port of Django's `strip_tags`. Pure Rust: what a work item's description goes through before it is kept |
| `usecases/` | `thrust-core` | Domain rules and use-cases, the Authorizer and the `Scope` proof types. Depends on `ports` and `types` only |

The dependency direction is `types`, then `ports`, then `usecases`; `html` depends on none of them. Which of the three a change belongs in:
`docs/src/contributing/crates.md`.
