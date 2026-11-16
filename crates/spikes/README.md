# crates/spikes/

Spikes that are workspace members (ADR-0009 rule 5).

| Directory | Package | What it is |
|---|---|---|
| `editor-worker/` | `thrust-editor-worker` | Spike 0006's measurement Worker: `thrust-editor-doc` inside workerd, for its size and CPU. No workflow or script builds or deploys it (`workers_dev = false`) |

## Here or in `probes/`?

- **Here**, when the spike must build against the workspace's lockfile, so that it measures the dependency versions the
  product builds with.
- **`probes/` or `tools/`**, with its own `[workspace]`, when it must not: it pulls a dependency the workspace bans or
  conflicts with (ADR-0044's ORM ban, for one; `probes/sqlx-wasm` is that case).

No product crate depends on a crate in `spikes/`.
