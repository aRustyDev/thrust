# crates/testing/suites/

Black-box suites: each tests a running target from outside, over its public interface. What belongs in `testing/` and
what does not: [`../README.md`](../README.md).

| Directory | Package | What it is |
|---|---|---|
| `api/` | `thrust-contract-tests` | ADR-0121's HTTP contract suite, run unchanged against every target by `scripts/contract.sh` |
| `load/` | `thrust-load-tests` | The load tests: latency (one request at a time and eight at once) and the 100k-row scale probe, as nextest tests named `load_`, run by `scripts/contract.sh <target> bench` and `scale` with the `load` profile |

`system/`, `acceptance/` and `security/` join them when each has code. Stress, spike and soak tests join `load/` when each has a
workload.
