---
status: accepted
date: 2026-11-15
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0009: Crate directory layout

## Context and Problem Statement

ADR-0005 put the Rust backend in a Cargo workspace under `crates/`, with crates named `thrust-*`. Eighteen crates now sit
flat in that directory (`crates/thrust-<name>/`), and the flat list hides what each one is: data types, port traits,
use-cases, adapters, HTTP surfaces, the two composition roots, a spike's measurement Worker, a black-box test suite and a
test fixture all look alike. It also has no place for what is planned: more programs (a CLI, a TUI, the Dioxus desktop,
web and mobile clients of ADR-0013, the edge Worker of #195, the collab Worker root of ARCHITECTURE §2a), more black-box
suites (system, acceptance, security, load) and profiling workloads.

This ADR decides the directory layout under `crates/`. It refines ADR-0005's layout line ("crates prefixed `thrust-*`
under `crates/`"), which still holds: every crate keeps its `thrust-*` package name and stays under `crates/`.

## Decision Drivers

* A crate's directory should say which layer it belongs to, so the dependency direction (types, then ports, then
  use-cases, then adapters and HTTP surfaces, then programs) is visible in the tree and checkable by directory.
* All tests run on cargo-nextest, and nextest filtersets select by package name, binary name, binary kind, test name and
  the dependency graph, never by file path (`cargo nextest help filterset`, nextest 0.9.140). So the layout is for
  people, and routing has to stay on names.
* Moving a crate must not rename its package: `release-plz.toml` (`changelog_include`), the filtersets in
  `.config/nextest.toml`, `-p` flags in the scripts and every `use thrust_*::` path key on package names.
* ADR-0121's contract suite stays one suite, run unchanged against every target.
* AGENTS.md rule 5: a crate's tests stay in that crate, because cargo-mutants runs only the mutated package's tests.
* No stub crates: a directory is created with its first real code (the `planning` skill's rule for plans, applied to
  crates; an empty crate is also a workspace member that CI builds).

## Considered Options

* Option 1: Keep the flat `crates/thrust-<name>/` layout.
* Option 2: Group library crates by layer; programs flat at the top of `crates/` **(chosen)**.
* Option 3: Group library crates by layer, and programs under an `apps/` group.

## Decision Outcome

Chosen option: Option 2, by the owner on 2026-11-15, because it shows the layering without moving the programs a level
down, and leaves a named place for every planned crate without creating any of them now.

The layout, with today's crates:

```text
crates/
├── adapters/                   README.md
│   ├── cloudflare/             thrust-adapters-cloudflare
│   ├── memory/                 thrust-adapters-memory
│   ├── oidc/                   thrust-adapters-oidc
│   ├── postgres/               thrust-adapters-postgres
│   └── selfhosted/             thrust-adapters-selfhosted
├── api/                        README.md
│   ├── core/                   thrust-api
│   ├── v1/                     thrust-api-v1
│   ├── compat-plane/           thrust-compat-plane
│   └── plane-types/            thrust-plane-types
├── core/                       README.md
│   ├── ports/                  thrust-ports
│   ├── types/                  thrust-contracts
│   └── usecases/               thrust-core
├── editor/                     README.md
│   └── doc/                    thrust-editor-doc
├── spikes/                     README.md
│   └── editor-worker/          thrust-editor-worker
├── testing/                    README.md
│   ├── fixtures/
│   │   └── oracle/             thrust-oracle-fixture
│   └── suites/
│       └── api/                thrust-contract-tests
├── server/                     thrust-server
├── worker/                     thrust-worker
└── README.md
```

The rules that come with it:

1. **A directory under `crates/` is a crate or a group, never both.** A group holds crates or further groups and has a
   `README.md` naming its members and what belongs in it. No crate contains another crate.
2. **Package names do not change.** Directories drop the `thrust-` prefix and may differ from the package name
   (`core/types` is `thrust-contracts`). A group is selectable as one by a filterset only when its packages share a name
   prefix: `package(thrust-adapters-*)` works today; the `core/` group is selected by listing its packages.
3. **Programs sit flat at the top of `crates/`**: `server/` and `worker/` today. Reserved names, each created with its
   first real code: `cli/`, `tui/`, `desktop/`, `webui/`, `mobile/` (ADR-0013), `edge/` (the edge Worker of #195),
   `collab/` (the collab Worker root, `thrust-collab-do` in ARCHITECTURE §2a). `crates/README.md` lists them.
4. **`core/` is the inside of the hexagon**: `types/` (data contracts), `ports/` (port traits), `usecases/` (domain and
   use-cases). Nothing in `core/` depends on a crate outside it.
5. **`spikes/` holds a spike that must build against the workspace's lockfile**, so that it measures the dependency
   versions the product builds with. A spike that must not, because it pulls a dependency the workspace bans or
   conflicts with (ADR-0044's ORM ban, for one), keeps its own `[workspace]` under `probes/` or `tools/`. No product
   crate depends on a crate in `spikes/`.
6. **`testing/` holds crates that exist only for testing and never ship**: no crate depends on one except as a
   dev-dependency. `suites/` holds black-box suites that test a running target from outside (`api/` is ADR-0121's
   contract suite; `system/`, `acceptance/`, `security/` and `load/` join it when each has code). `fixtures/` holds
   test-data libraries. `profiling/` will hold in-process profiling workloads that span crates, run under nextest
   wrapper scripts; it is created with its first workload.
7. **What does not go in `testing/`**: unit tests (in `src/`, under `#[cfg(test)]`); a crate's integration, property,
   model and regression tests (its own `tests/`, rule 5); a crate's micro-benchmarks (its own `benches/`); mutation
   testing (a tool run over crates, configured in `.cargo/mutants.toml` and `scripts/mutants.sh`); nextest profiles
   (`.config/nextest.toml`); the Playwright journeys (`e2e/` at the repository root, a pnpm project, not a crate).
   `testing/README.md` carries this classification, so the kinds of test that have no directory yet still have a home.
8. **Tests are routed by name, not by folder.** Test kinds are test-name prefixes (`prop_`, `model_`, `parity_`,
   `oracle_`, `fuzz_replay_`, and `regression_` for a pinned regression, added to AGENTS.md rules 4 and 9 by this
   decision's plan); groups of crates by package-name prefix.

Alternatives considered and not taken, within the chosen option:

| Proposal | Outcome | Why |
|---|---|---|
| An `apps/` group for the programs | rejected (owner) | Option 3; programs stay at the top level |
| Black-box suites in a root `tests/` | rejected | In Cargo `tests/` is one package's integration-test directory; the root `Cargo.toml` is a virtual manifest, so a root `tests/` would hold hand-listed members under a borrowed name, invite the misplacement rule 5 forbids, and fall outside what `plan-ci.py` and `check-deps.sh` treat as crates |
| One directory per test kind under `testing/`, created now (`e2e`, `integration`, `models`, `mutants`, `benches`, `profiles`, `security`, `load/{stress,spike,soak}`, `database`, `api`, `system`, `acceptance`) | narrowed | Most would be stub crates; `models`, `integration`, `database` and `benches` belong in the crate under test (rule 5); `mutants` and `profiles` are not test code; `e2e` is a pnpm project. The list survives as the classification in `testing/README.md` |
| Per-crate `tests/contracts/<contract>/` | not adopted | Each adapter testing its own port is ADR-0121's Option 2, rejected there ("drift guaranteed; defeats ADR-0002"). Reopening it needs a new ADR |
| Per-crate `tests/regressions/` with `*_test.rs` files | replaced | Cargo compiles only `tests/*.rs` and `tests/<dir>/main.rs`, so a file in a subdirectory runs only if a module declaration reaches it; `_test.rs` is Go's convention; a folder reaches a filterset only through a module or binary name. A `regression_` name prefix is selectable in every crate with one filterset |
| `profiles/` | renamed `profiling/` | "profile" already means a nextest profile in this repo |
| `editor/worker` or `workers/editor` for `thrust-editor-worker` | `spikes/editor-worker` | It is spike 0006's measurement Worker: no workflow or script builds or deploys it, and its `wrangler.toml` sets `workers_dev = false` |
| Renaming `thrust-contract-tests` to `thrust-suite-api` | declined (owner) | Select it with `package(thrust-contract-tests)` |

### Consequences

* Good, because the tree shows each crate's layer, and dependency rules can be checked by directory (rules 4 to 6).
* Good, because every planned program, suite and profiling workload has a named place, and none is created early.
* Good, because package names are unchanged, so release-plz, the nextest filtersets, the `-p` flags and the `use` paths
  are unaffected by the move.
* Bad, because a directory name can differ from its package name, so finding a crate's directory from its name needs
  `cargo metadata` or the group READMEs.
* Bad, because the move is one large change: on `main` at `ce84d85`, 45 files outside the docs name a crate path, 27
  documentation files do, and any branch alive across the move must be rebased onto it.
* Bad, because crates sit at different depths (`crates/server/` is two levels down, `crates/testing/suites/api/` four),
  so a relative path that climbs out of a crate depends on where the crate is. The plan replaces those climbs with a
  workspace-root variable before the move.
* Neutral: `thrust-api`'s manifest calls it ARCHITECTURE §2a's `thrust-api-core`, so it takes `api/core/`. §2a also
  describes a core REST surface generated at M4; if that becomes a crate of its own, it needs another name, or
  `thrust-api` moves (for example to `api/router/`).
* Neutral: accepted ADRs and dated plan documents keep citing the old paths. They are records of what was true then and
  are not rewritten.

### Confirmation

* The move's pull request compares `cargo metadata` before and after: the same 18 packages with the same dependency
  edges, kinds and features, differing only in `manifest_path`.
* After the move, `git grep 'crates/thrust-'` outside `.agents/plans/` and `CHANGELOG.md` finds nothing.
* A layout guard in CI's `deps` job checks that every directory under `crates/` is a workspace member or a group with a
  `README.md`, that every member is under `crates/`, and that no crate has a normal or build dependency on a crate under
  `crates/testing/` or `crates/spikes/`. Each check ships with a self-test that plants a violation and must fail
  (AGENTS.md rule 8).

## More Information

The work: `.agents/plans/crates-layout/`. Related: [[ADR-0005]] (repository and package layout), [[ADR-0013]] (Dioxus
multi-target clients), [[ADR-0121]] (one contract suite, N adapters), [[ADR-0044]] (no ORM), ARCHITECTURE §2a,
`docs/src/contributing/crates.md`, and the filterset reference at <https://nexte.st/docs/filtersets/reference/>.
