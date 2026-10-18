# In-memory adapters, real Postgres, and flake hunting

Evidence: `crates/thrust-adapters-memory/src/lib.rs`, `crates/thrust-adapters-selfhosted/{Cargo.toml,tests/pg_adapter.rs}`,
`crates/thrust-adapters-postgres/src/lib.rs`, `scripts/flake-hunt.sh`, `scripts/contract.sh`.
Tags are defined in `SKILL.md`.

## In-memory adapters instead of mocks

`thrust-adapters-memory` implements every port, has no runtime dependency and compiles for wasm32
and native, so the same fake serves the `native-memory` and `worker-memory` matrix cells. Because the
suite is HTTP-level, the fake is exercised through the real router, auth, tenancy and error mapping:
nothing in the path is replaced by a canned return value.

Properties the fakes reproduce on purpose: transactions buffer writes and apply them on `commit`
(`MemoryTx`), uniqueness is re-checked under the lock at commit, the queue delivers in-process through
the `Background` port to a late-bound consumer (`ConsumerSlot`, which breaks the App-Queue construction
cycle), and activity inserts are idempotent on `event_id`.

### A fake can drift from the real thing: an example to re-check

| | in-memory adapter | Postgres adapter |
|---|---|---|
| `Tx::allocate` | bumps the counter immediately; doc comment: "`allocate` is *not* transactional (like a Postgres sequence, a rollback leaves a gap; documented in ADR-0043)"; `rollback` is a no-op | an `INSERT ... ON CONFLICT DO UPDATE` inside the transaction, so a rollback restores the counter |
| pinned by | nothing at the HTTP level **[derived]** | `pg_adapter.rs::rollback_is_atomic_and_unique_violation_is_a_conflict`: "rolled-back allocation must not leave a gap" |

Read both sources before relying on this row; it describes the code as read and may have been
reconciled since. The HTTP suite asserts statuses after a failed duplicate create, not the next
sequence number, so it does not pin either behaviour **[derived]**. Which one is intended is an
ADR-0043 question (not restated here). The general lesson: **where a fake and the real adapter can
observably differ, pin the behaviour in a test that runs against the real one, and decide which is
right.** A fake is only as good as the assertions that compare it with reality.

## Real Postgres with testcontainers

`crates/thrust-adapters-selfhosted/tests/pg_adapter.rs`. The crates and versions are in
`[dev-dependencies]` of `crates/thrust-adapters-selfhosted/Cargo.toml` (`testcontainers` and
`testcontainers-modules` with the `postgres` feature). Needs a running Docker daemon, and pulls the
Postgres image on first use, which needs network access.

```rust
let container = Postgres::default().start().await.expect("start postgres container");
let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
// the module's default credentials are a throwaway for this container only
let url = format!("postgres://<user>:<password>@127.0.0.1:{port}/<database>");
```

The schema is applied with `thrust_adapters_postgres::migrate`, which runs the very SQL the adapter ships
(no hand-copied DDL). Each test starts its own container and holds it in the test's `Env` until the end.

What the adapter tests pin (and the HTTP suite cannot see): tenancy answers (`NoSuchProject` for another
tenant), concurrent allocations are unique and contiguous, rollback atomicity and unique-violation ->
`Conflict`, an abandoned transaction must not poison the pool (more iterations than pool slots, inside a
`tokio::time::timeout` so a wedge fails instead of hanging), soft delete frees the external key but never
the sequence number, and the outbox row is atomic with its entity.

Note the two Postgres mechanisms: `scripts/contract.sh` starts its own Postgres container with a random
port and a per-run password forwarded by environment (never argv) for the HTTP matrix; testcontainers is
used only by the adapter tests. A connection-pool bug is caught by the second kind (and by its
`discard-noop` mutant, see `must-fail-controls.md`), not by the first.

## `scripts/flake-hunt.sh`

```sh
scripts/flake-hunt.sh <target> <pass> <iterations>     # default 5
```

Re-runs one contract cell N times with a separate `THRUST_LOGDIR` per iteration
(`target/flake-hunt/<target>-<pass>/iter<N>*`), prints `iter i: exit=S` and finally
`failures: F / N`.

- **Read the `failures:` line, not the exit status.** The script's last command is the `echo`, so it
  exits 0 whatever happened; a wrapper that branches on its exit code will call every run green.
- Size N to the rate you are hunting: at a 1-in-4 failure rate, N = 10 misses it with probability
  0.75^10, about 6%. One green run proves nothing about a flake. The Worker root's body-drain bug showed
  at roughly that rate **[spike]** (see skill `rust-on-workers`).
- Keep the per-iteration logs; a flake you cannot read after the fact is not diagnosable.
- Serial only (shared build directory), same as `contract-all.sh`.
- Do not add automatic retries to hide a flake. If a runner retries (nextest does:
  `nextest-and-mutants.md`), turn on the setting that still fails the run on a flaky pass.
