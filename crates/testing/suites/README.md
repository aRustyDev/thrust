# crates/testing/suites/

Black-box suites: each tests a running target from outside, over its public interface. What belongs in `testing/` and
what does not: [`../README.md`](../README.md).

| Directory | Package | What it is |
|---|---|---|
| `api/` | `thrust-contract-tests` | ADR-0121's HTTP contract suite, run unchanged against every target by `scripts/contract.sh` |

`system/`, `acceptance/`, `security/` and `load/` join it when each has code.
