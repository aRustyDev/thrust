# Glossary

Testing terms as this repo uses them. Where a word has more than one meaning here, every meaning is listed: say
which one you mean.

## Oracle

A reference implementation whose behaviour defines "correct". Three different oracles exist:

| Oracle | What it is | Where | Used by |
|---|---|---|---|
| **Legacy-TypeScript oracle** | the original TypeScript run in Node, side by side with Rust | `oracle_` nextest tests (none yet); templates in `rust-test-logic` | [differential](techniques/differential.md) |
| **Editor/sanitiser oracle (spike 0006)** | Plane's real editor packages (TipTap, Yjs) and Python `nh3`, run offline to produce corpora | `tools/spike-0006/` | [golden](techniques/golden-fixtures.md), [differential](techniques/differential.md) |
| **Plane v1 oracle** | a pinned upstream Plane deployment the public API must match | ADR-0122 (planned) | [differential](techniques/differential.md) |

`ts-to-rust` also replays **recorded** responses of a TypeScript or Python reference API against a port; that is a
golden test whose fixtures are HTTP exchanges.

## Golden, differential, contract

- **Golden (parity) test**: compares output with **committed** expected output. Hermetic. Prefix `parity_`.
- **Differential test**: runs the same, usually generated, inputs through Rust and an oracle and compares. Needs the
  oracle or its freshly generated output. Prefix `oracle_` when live.
- **Contract test**: asserts behaviour that every implementation of an interface must share; here, the HTTP suite in
  `crates/thrust-contract-tests` run against every target.

## Exception register

A committed list of known, accepted differences from an oracle, so that "matches the oracle" can mean "the failure
set equals the register". There are three, one per oracle family; never edit a fixture instead of adding an entry.

| Register | For | Defined in |
|---|---|---|
| `tests/oracle/exceptions.json` | a TypeScript-to-Rust port | `ts-to-rust` |
| `oracle/exceptions.yaml` | the Plane v1 API | ADR-0122 (planned) |
| `.agents/plans/contrail/DEVIATIONS.md` | deliberate product deviations from Plane, each pinned by a test | the Contrail plan |

## Other terms

- **Target**: one runnable deployment the contract suite is pointed at: `native-memory`, `native-pg`,
  `worker-memory`, `worker-cf`, or a deployed Worker (stage 2).
- **Adapter set**: the group of adapters a target is built with: memory, postgres (self-hosted), or cloudflare.
- **Pass**: a named subset of the contract suite selected by test-name prefix in `scripts/contract.sh` (`main`,
  `sweeper`, `panic`, `worker`, …), run against a fresh target.
- **Cell**: one target × pass combination; `scripts/contract-all.sh` runs 16.
- **Control**: an act that proves a test can fail; see [Evidence and controls](evidence-and-controls.md).
- **Vacuous pass**: a green result that checked nothing: zero tests selected, the wrong target, an empty corpus.
- **Planted mutant**: a `mutant_*` cargo feature that builds a deliberately broken adapter for `scripts/mutants.sh`.
- **Killed / MISSED**: a mutant the tests caught / did not catch.
- **Hermetic**: depends on nothing outside the process and a temp directory: no network beyond `127.0.0.1:0`, no
  wall clock, no sleeps ([Rule 6](rules.md)).
- **Stage 2**: running against real Cloudflare infrastructure in an owner-provided sandbox, as opposed to local
  workerd.
- **Test home**: the column in `.agents/plans/contrail/TRACEABILITY.md` naming where a requirement's acceptance
  evidence lives.
