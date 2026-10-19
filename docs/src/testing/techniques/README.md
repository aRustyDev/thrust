# Techniques

One page per technique, all with the same headings, so an agent can jump straight to the part it needs. The status
legend is in the [chapter README](../README.md#status-legend).

| Technique | Status | Prefix | Owning skill |
|---|---|---|---|
| [Example and parameterized](example-and-parameterized.md) | live (`thrust-core`) | none | `rust-test-logic` |
| [Property](property.md) | live (`thrust-core`) | `prop_` | `rust-test-logic` |
| [Model-based](model-based.md) | live (`thrust-core`) | `model_` | `rust-test-logic` |
| [Golden fixtures](golden-fixtures.md) | live | `parity_` | `rust-test-logic`, `ts-to-rust` |
| [Differential](differential.md) | live (editor-doc); planned (v1) | `oracle_` | `rust-test-logic` (partly) |
| [Contract suite](contract-suite.md) | live | pass prefixes | none |
| [Real backends](real-backends.md) | live | none | none |
| [Fuzz](fuzz.md) | live (replay without a fuzzer) | `fuzz_replay_` | `rust-test-fuzz` |
| [Mutation](mutation.md) | live (planted; cargo-mutants on `thrust-core`) | n/a | `rust-test-mutation` |
| [App and end-to-end](app-and-e2e.md) | template-only; browser planned | `e2e_` | `rust-test-app` |
| [Architecture gates](architecture-gates.md) | live | n/a | none |
| [Performance](performance.md) | live, measurement only | n/a | none |

This table summarises; the `Status:` line on each page is the authority.

## Page template

```
# <Technique>
Status: … · Gate: <command that fails a build, or "none">
## Catches            defect-matrix ids, P (primary) or B (backstop)
## Use when / Not when
## Name and location  prefix, crate and path, fixture directory
## Run                exact command, tagged [verified …] / [unverified: not run] / [derived]
## Required controls  links into Evidence and controls
## Owning skill
## In-repo examples   paths, or "none yet"
## Target             one line and the owning ADR or gap
```
