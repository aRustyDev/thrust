# Sources and provenance: rust-on-workers

## Provenance

- **Ground truth:** the repo's spike branch `feat/spike-0001-rust-on-workers`, verified at commit
  `620e2a1` (read 2026-10-18). Cited paths are repo-relative; re-check them when the Worker crate
  layout changes. ADRs are cited by id only: ADR-0002, 0007, 0033, 0046, 0070, 0072, 0082.
- **External sources:** workers-rs **0.8.7** source (`worker/src/send.rs`, `worker/src/lib.rs`,
  `worker-macros/src/lib.rs`) and the behaviour of locally installed tools, read/run 2026-10-18:
  rustc 1.98.1 (Homebrew) and 1.99.0 (rustup stable), wrangler 4.123.0 with its bundled workerd,
  worker-build 0.8.7, wasm-bindgen CLI 0.2.129. Cloudflare's own docs were not consulted; the skill
  points to them rather than restating them.
- All prose is original; API names are used for accuracy. Last reviewed 2026-10-18.

## Tag legend (used in the references)

- `[re-run]` reproduced locally, offline, from a copy of the spike tree or a scratch crate.
- `[spike]` recorded by the spike, not re-run.
- `[derived]` reasoned from this repo's code, not tried.
- `[unverified: <tool> not installed]` the command needs a tool that was not available.

## Repo files cited

`Cargo.toml`, `Cargo.lock`; `scripts/{rs,check-deps.sh,worker-dev.sh,contract.sh,contract-all.sh,
startup-check.sh,stage2-latency.sh}`; `crates/thrust-worker/{Cargo.toml,wrangler.toml,
wrangler.memory.toml,entry.mjs,src/lib.rs,src/durable.rs,src/memory.rs,src/probes.rs}`;
`probes/send-shape/*`, `probes/sqlx-wasm/*`; `crates/thrust-ports`, `thrust-api`,
`thrust-adapters-{cloudflare,postgres,selfhosted,memory}`, `thrust-server`, `thrust-contract-tests`.

## Reproduced locally (tag `[re-run]`), offline, from a copy of the spike tree

| Claim | Result |
|---|---|
| Send-shape matrix (3 shapes x 2 targets) | all six cells match the probe's logs |
| `!Send` value held across an await under `#[async_trait]` | "future cannot be sent between threads safely" |
| Homebrew rustc lacks wasm32 std | `E0463 can't find crate for std` |
| rustup `rust-lld` dyld fault and its `DYLD_FALLBACK_LIBRARY_PATH` fix | reproduced and fixed |
| wasm32 `panic-strategy` | `abort` |
| Local workerd accepts 2026-08-18, rejects 2026-08-19; `--version` prints 2026-08-11 | reproduced, exact error captured |
| `scripts/check-deps.sh` passes; tokio is in the wasm graph via tokio-postgres/worker | reproduced |
| Positive control for the guard (`-p thrust-server -i tokio` prints a tree) | reproduced |
| `cargo check -p thrust-worker --no-default-features --target wasm32-unknown-unknown` | exit 0 |
| `scripts/rs worker-build --release . --no-default-features` (control, `strip = "debuginfo"`) | exit 0 |
| Same command with only `strip = true` | exit 1: "failed to generate catch wrappers ... externref table required for catch wrappers" |
| `scripts/startup-check.sh` | completes without a login, writes a `.cpuprofile` |
| sqlx (postgres, no runtime feature) on wasm32 | fails in `home 0.5.12` |

## Carried from the spike, not re-run (tag `[spike]`)

- `rustup run stable cargo` does not fix Fault 1; the SIP explanation for Fault 2.
- The roughly 1-in-4 failure rate without body draining, the `env -i` necessity, the close-code 1006
  throw, the shared-build-dir reload, the Queue payload-as-`Map` loss, the workerd panic blast radius
  (all need a running workerd stack).
- The explanation that `strip = true` drops the `target_features` section (the failure itself is `[re-run]`).
- The sqlx probe with `--features tokio`.
