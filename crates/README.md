# crates/

The Rust workspace's crates, laid out by ADR-0009 (`.agents/plans/contrail/adrs/architecture/0009-crate-directory-layout.md`).

A directory here is a **crate** or a **group**, never both, and no crate contains another. A group holds crates or
further groups, and its `README.md` names its members and says what belongs in it. Package names keep the `thrust-`
prefix and do not change when a crate moves; directories drop it, and may differ from the package name
(`core/types/` is `thrust-contracts`). Filtersets, `-p` flags, `release-plz.toml` and `use` paths all key on package
names, so they are independent of this tree. To find a crate's directory from its name, use `cargo metadata` or the
group READMEs. `scripts/check-layout.sh` (CI's `deps` job) checks these rules, and that each group's README names every
directory in it.

| Directory | What it holds |
|---|---|
| [`core/`](core/README.md) | The inside of the hexagon: data types, port traits, use-cases. Depends on nothing outside `core/` |
| [`api/`](api/README.md) | The HTTP surfaces and the router they compose into |
| [`adapters/`](adapters/README.md) | Implementations of the ports, one crate per adapter set |
| [`editor/`](editor/README.md) | The editor document |
| [`spikes/`](spikes/README.md) | Spikes that must build against the workspace's lockfile |
| [`testing/`](testing/README.md) | Crates that exist only for testing and never ship |
| `server/` | `thrust-server`: the native composition root (axum/tokio) |
| `worker/` | `thrust-worker`: the Cloudflare Worker composition root (wasm32) |
| `preview-provisioner/` | `thrust-preview-provisioner`: restricted internal database provisioning Worker |
| `preview-migration-executor/` | `thrust-preview-migration-executor`: closed-by-default preview migration Worker |
| `edge/` | `thrust-edge`: the Worker on the shared hostname (ADR-0111) |

## Programs

Programs sit flat at this level, with no group. These names are reserved, and each directory is created with its first
real code, never as a stub:

| Name | For |
|---|---|
| `cli/` | a command-line client |
| `tui/` | a terminal client |
| `desktop/`, `webui/`, `mobile/` | the Dioxus clients (ADR-0013) |
| `collab/` | the collab Worker's root (`thrust-collab-do`, ARCHITECTURE §2a), not before M5 |

## Adding a crate

`docs/src/contributing/crates.md` ("Adding a crate") has the steps. In short: pick the group for the crate's layer, or
this level for a program, add the crate to that group's `README.md`, and add its directory to `members` in the root
`Cargo.toml`.
