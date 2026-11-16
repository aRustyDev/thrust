# measure-zxcvbn

The harness, tests, results and analysis behind `.agents/plans/contrail/research/0015-zxcvbn-worker-cost.md`: what Plane's password rule (zxcvbn, score 3 or
more on the first and the last 72 characters) would cost the Worker, which is the open question of deviation D-023. The answer is in that note and in
the D-023 row of `DEVIATIONS.md`; this folder is how it was got, so that it can be checked or run again.

This is test tooling, not product code. No crate imports it (ADR-0006 register). **The `zxcvbn` crate is not a dependency of anything in the workspace and
must not become one by way of this folder:** `scripts/check-deps.sh` forbids what it pulls in on wasm32 (`wasm-bindgen`, `js-sys`, `web-sys`) in
`thrust-core`'s graph, and the measurement showed it fails with 15 hits (`results/2026-11-14/check-deps.txt`). The variant that calls it is a patch
(`patches/`), applied to a working tree for a build and then reverted, never committed.

The owner decided on 2026-11-14 to leave D-023 as it is (the hand-written rule). Nothing here changes that: it is the evidence for the decision.

| Path | What it is |
|---|---|
| `patches/variant-called.patch` | the variant: `zxcvbn = "=3.1.1"` in `thrust-core`, and Plane's rule called from `password_is_strong` |
| `patches/variant-called-nodefault.patch` | the same with zxcvbn's default features off (the `builder` feature is build-time only: 4,476 B of difference) |
| `patches/variant-uncalled.patch` | the control: the crate referenced by a `pub fn` that nothing calls. It should add nothing; it added 2,633 B (0.053%), cause not established |
| `bundle/sizes.py` | bytes, `gzip -9` bytes and sha256 of one `worker-build` output, and a count of dictionary needles in the wasm (the positive control) |
| `bundle/make-needle.py` | cuts the needles (12 consecutive entries from the middle of two of zxcvbn's lists) from the crate's source |
| `bundle/startup-repeat.sh` | repeats `scripts/startup-check.sh` and prints which bundle each run profiled |
| `native/` | a detached crate (own `Cargo.lock`): the native timing of the first call and the heap it leaves (`run.sh`), and `cargo test` assertions on the probe passwords |
| `local-workerd/probe.sh` | starts the Worker on local workerd and asserts six statuses; exits non-zero if one differs |
| `local-workerd/cold-runs.sh` | N cold starts of the Worker on local workerd, timing the first zxcvbn request against warm ones (wall clock) |
| `cloudflare/measure-cpu.sh` | the deployed measurement: throwaway Workers, a tail consumer and a KV namespace, `cpuTime` read from the platform |
| `cloudflare/summary.py`, `cold-analysis.py` | the analysis of its `client.tsv` and `kv.json` |
| `cloudflare/verify-teardown.sh` | checks that every Worker and the KV namespace it made are gone |
| `results/2026-11-14/` | what was measured; see below |

It reuses `crates/worker/measure/tail.mjs` (the tail consumer of `scripts/measure-argon2.sh`, issue #146) and follows that script's pattern.

## What was measured, and how to run it again

Build with `scripts/rs`, which puts rustup's toolchain (with the wasm32 standard library) in front of Homebrew's. Reset a tree with
`git checkout -- crates/core/usecases Cargo.lock` after a variant.

**Bundle size** (`results/2026-11-14/sizes.txt`, `startup.txt`). In `crates/worker`: `../../scripts/rs worker-build --release . --no-default-features` (the
command of `deploy.yml`), then `uv run --no-project python -I ../../tools/measure-zxcvbn/bundle/sizes.py <label> build @../../tools/measure-zxcvbn/results/2026-11-14/needles.txt`.
Apply a patch from `patches/`, build again, run it again. `scripts/startup-check.sh wrangler.toml` (or `bundle/startup-repeat.sh`) for startup: it **re-runs the
build**, so the bundle size it prints is the only proof of which source it profiled.

**The first call, natively** (`native.txt`). `native/run.sh 20`: one process per run, so every first call is cold. `cargo test --manifest-path
tools/measure-zxcvbn/native/Cargo.toml` runs the assertions on the probe passwords (below).

**The first call, on local workerd** (`local-workerd.txt`). Apply `patches/variant-called.patch`, then `local-workerd/probe.sh [port]` (the test) and
`local-workerd/cold-runs.sh <runs> <port>` (the timing). Wall clock over loopback, not CPU, and not the platform.

**The first call, on Cloudflare** (`cloudflare/`). Needs a wrangler login with the `workers`, `workers_kv`, `workers_scripts` and `workers_tail` scopes and the
account id in `CLOUDFLARE_ACCOUNT_ID`; **it writes to that account**, so run it only on an account you may write throwaway Workers to. Build the memory adapter set
twice, the baseline from the unmodified tree and the variant with `patches/variant-called.patch` (`../../scripts/rs worker-build --release .` in
`crates/worker`, copying `build/` aside each time), take a needle from `results/2026-11-14/needles.txt`, and run
`BUILD_BASE=<dir> BUILD_VAR=<dir> NEEDLE=<line> CLOUDFLARE_ACCOUNT_ID=<id> tools/measure-zxcvbn/cloudflare/measure-cpu.sh 6`. Six cycles are 18 rounds, 18 new Workers
named `thrust-spike-zxcvbn-rNN`, a tail consumer and a KV namespace, about 15 minutes. Then `summary.py <outdir>`, `cold-analysis.py <outdir>` and `verify-teardown.sh 18`.

Safety, built into `measure-cpu.sh`: it refuses to start if the baseline's wasm holds the dictionary or the variant's does not; it refuses if any of its 19 Worker
names or the KV title already exists (a `wrangler deploy` on an existing name **overwrites** that Worker); it deletes only what it created; the operator token goes to
wrangler in a 0600 `--secrets-file` and to curl in the environment, never on an argv.

## Results (`results/2026-11-14/`)

| File | Made by | Note |
|---|---|---|
| `cloudflare/full-run/client.tsv`, `kv.json` | the run: the client's timings, the platform's `cpuTime` per request (666 requests, 18 rounds, the Woven account) | raw |
| `cloudflare/full-run/summary.txt`, `cold-analysis.txt` | `summary.py`, `cold-analysis.py` on that data | regenerated and checked (below) |
| `cloudflare/full-run/env.txt` | the run | paths replaced by `BUILD_BASE`, `BUILD_VAR` |
| `cloudflare/pilot-2/` | the second pilot (redeploying one Worker per round) | raw, with `summary.txt`; its `awk` check is below |
| `sizes.txt`, `startup.txt`, `native.txt` | the tools above, printed during the work | **transcribed verbatim from the terminal**, with the corrections noted in each file |
| `check-deps.txt` | `grep`, `sort`, `uniq` on the two `check-deps.sh` logs | generated; the raw logs name local paths and are not kept |
| `local-workerd.txt` | `probe.sh` and `cold-runs.sh` | the timings transcribed from the terminal; the two probe runs regenerated at `d16dbd6` |
| `needles.txt` | `make-needle.py` | |

Not kept: the wasm of each build (5 to 7 MB each: rebuild it, and compare the hash in `sizes.txt`), and the wrangler deploy logs (they name the account's `workers.dev` subdomain).

The pilot's version leak, from `client.tsv` (column 3 is the round, column 8 the version tag the response carried): `awk -F'\t' '$8 != "round-"$3' results/2026-11-14/cloudflare/pilot-2/client.tsv | wc -l` prints 23 of the 111 requests.

## Tests, and how each was shown to be able to fail

- **`native/tests/probe_passwords.rs`** (`cargo test`): the weak probes are accepted by the old rule and refused by zxcvbn, the strong one passes both, and Plane's
  rule reads the last 72 characters. The last test failed on its first construction (the alphabet three times over still scores 3), which is how it was shown to
  discriminate; the file says so.
- **`local-workerd/probe.sh`**: on the **unmodified tree it fails**, at exactly the two requests that only zxcvbn refuses (they answer 204); with the variant it
  passes. Both runs are in `local-workerd.txt`.
- **`measure-cpu.sh`'s guards**: handed the baseline as the variant, or the variant as the baseline, it exits 2 before any network call, and its teardown deletes nothing.
- **The analysis scripts** were run on the committed raw data and reproduce what was reported (`summary.txt` is byte-identical to the original output).
- **`verify-teardown.sh`** counts anything but "does not exist" (code 10007) as not gone, so a broken check fails safe.

## What the Cloudflare data can and cannot say

- The figure is the cold-cluster comparison in `cold-analysis.txt`: a request above 10 ms is a cold one (warm ones cost 1 to 4 ms in every arm; the result is the same
  at 5, 10 and 15 ms), and the first call is `median(vZ cold) - median(S cold)`.
- **The same-connection sequences are not used for the figure.** The control for them failed: on the arms where zxcvbn never runs, the first request of a sequence
  minus the later ones was 0 to 45 ms where 0 was expected, because a new connection sometimes reaches a fresh isolate and pays a cold start. They are in `client.tsv`
  and `summary.txt`.
- The second burst is only partly warm (12 to 27% of its requests were cold, against 35 to 64% of the first): `cold-analysis.txt` has the counts.
- The bootstrap resamples requests, so its interval is a little narrower than it should be; the per-round comparison in the same file does not have that problem.
- One account, one colo (not recorded), one session; `cpuTime` has a resolution of 1 ms; one 12-character password; the memory adapter set.
