# Dev loop, `compatibility_date`, and the two toolchain faults

Evidence: `scripts/rs`, `scripts/worker-dev.sh`, `scripts/contract.sh`, `scripts/contract-all.sh`,
`scripts/startup-check.sh`, `crates/thrust-worker/wrangler.toml`, `wrangler.memory.toml`. Tags:
**[re-run]** reproduced locally from a copy of the spike tree, offline; **[spike]** recorded by the
spike, not re-run. Detail: `SOURCES.md`.

## Always go through `scripts/rs` on macOS where Homebrew's rustc shadows rustup

```sh
scripts/rs cargo check --target wasm32-unknown-unknown
scripts/rs worker-build --release . --no-default-features    # from crates/thrust-worker
```

`scripts/rs` puts the rustup *stable* toolchain first on `PATH` and sets `DYLD_FALLBACK_LIBRARY_PATH`,
then `exec`s its arguments, so it works for any command. The two faults below hit **any** wasm32 build
on such a machine, including non-Worker ones (for example a Dioxus web build). Neither occurs when
`rustc` on `PATH` already is the rustup toolchain.

### Fault 1: `cargo`/`rustc` on PATH is Homebrew's and has no wasm32 std

```text
error[E0463]: can't find crate for `std`
  = note: the `wasm32-unknown-unknown` target may not be installed
  = help: consider downloading the target with `rustup target add wasm32-unknown-unknown`
```

**[re-run]** Homebrew rustc 1.98.1's sysroot `lib/rustlib` lists only `aarch64-apple-darwin`; the rustup
stable 1.99.0 sysroot also holds `wasm32-unknown-unknown`; Homebrew `cargo check --target
wasm32-unknown-unknown` fails with exactly the text above. The "obvious" fix is wrong: the script header
records that `rustup run stable cargo` does **not** help because cargo still resolves a bare `rustc`
through `PATH` and finds Homebrew's **[spike]**. Putting the rustup toolchain `bin` first does. The
`rustup target add` hint is also not the fix: the target is already installed under rustup.

### Fault 2: rustup's `rust-lld` cannot find `libLLVM.dylib`

```text
dyld: Library not loaded: @rpath/libLLVM.dylib
```

**[re-run]** Running the toolchain's `rust-lld` directly reproduces it: `libLLVM.dylib` sits in
`<toolchain>/lib`, which is not on `rust-lld`'s rpath. With `DYLD_FALLBACK_LIBRARY_PATH=<toolchain>/lib`
it loads. The script sets the variable *inside* itself, after `/bin/sh` has started, because System
Integrity Protection strips `DYLD_*` from the environment of protected binaries such as `/bin/sh`
**[spike]**. Practical consequence: exporting the variable in your shell and then running `sh script`
does not work; wrap the command with `scripts/rs`.

A link failure that mentions `libLLVM.dylib` is this fault, not a corrupt toolchain.

## `wrangler dev` loop

Build indirection: `wrangler.toml` has `[build] command = "../../scripts/rs worker-build --release .
--no-default-features"`, so wrangler never needs to know about Rust and the rustup toolchain is used
(the path is relative to `crates/thrust-worker`). The memory-only config `wrangler.memory.toml` builds
with default features and `main = "build/worker/shim.mjs"`.

`scripts/worker-dev.sh` (needs `THRUST_PORT`; the contract script drives it) starts local workerd:

- config: `THRUST_ADAPTERS=memory` selects `wrangler.memory.toml`, anything else `wrangler.toml`;
  `scripts/contract.sh` passes `cloudflare` for the Cloudflare set (read the `case` in the script,
  not its header comment);
- `wrangler dev --local --test-scheduled --persist-to <fresh state dir> --var ...`, no login, no remote
  bindings, state dir deleted at start;
- started under `env -i` with an explicit minimal environment, because
  `CLOUDFLARE_INCLUDE_PROCESS_ENV=true` turns the *whole* process environment into Worker bindings
  (wrangler lists every environment variable name) **[spike]**;
- secrets (`THRUST_ADMIN_TOKEN`, the Postgres URL) travel in the environment, never in a file or argv.

**Do not run two cells in parallel.** They share `crates/thrust-worker/build`; a second wrangler rebuild
reloads the first one's worker **[spike]** (`scripts/contract-all.sh` is serial for this reason). Ports
are chosen at random because fixed ports collide with other local processes. Only signal processes the
script started; never `pkill` by name.

`scripts/startup-check.sh` runs `wrangler check startup` (it runs the `[build]` command first) and
profiles global-scope execution locally, writing a `.cpuprofile` under `target/startup/`. **[re-run]**
It completes from a copy of the spike tree without a login. `scripts/stage2-latency.sh` sends real
requests to a **deployed** Worker: never run it without an owner-provided sandbox and `THRUST_BASE_URL`
set on purpose.

## `compatibility_date` is capped by the local workerd

The limit belongs to the workerd bundled with your wrangler, so it moves when wrangler is upgraded.
For wrangler 4.123.0 the newest accepted date is 2026-08-18 (recorded in `wrangler.toml`).
**[re-run]** Against that bundled workerd, a worker config with `2026-08-18` starts and `2026-08-19`
is rejected:

```text
service main: This Worker requires compatibility date "2026-08-19", but the newest date supported by this server binary is "2026-08-18".
```

`workerd --version` printed `workerd 2026-08-11` for the same binary, so the version string is **not**
the accepted maximum: read the date out of the error message. Both wrangler configs in
`crates/thrust-worker` set a date inside the limit. When you upgrade wrangler, re-measure by trying a
date; do not assume. A deployed Workers runtime may accept newer dates than local workerd; that was not
tested.

## Common failures, one line each

| Symptom | First thing to check |
|---|---|
| `can't find crate for `std`` on wasm32 | not running under `scripts/rs` (Fault 1) |
| `Library not loaded: @rpath/libLLVM.dylib` | not running under `scripts/rs` (Fault 2) |
| `newest date supported by this server binary` | lower `compatibility_date` to the date in the message |
| `externref table required for catch wrappers` | `strip = true` in the release profile |
| Suite fails about 1 in 4 runs under `wrangler dev` | body not drained before routing (see `build-and-features.md`) |
| `VACUITY GUARD` on `/_info` right after start | the worker may still be reloading; re-run the cell |
