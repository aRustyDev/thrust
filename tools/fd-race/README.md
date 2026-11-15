# fd-race

A std-only program that asks one question: when many threads spawn children with piped stdout, is a pipe's EOF ever held open by
a child that another thread spawned at the same moment? It is the evidence for D-19 in `docs/src/testing/status.md`: nextest
reported a test `LEAK` (a pass whose pipes were still open 200 ms after its process exited) that spawns nothing and has one thread.

Not a test and not part of the workspace (its own `[workspace]` table, as `tools/measure-zxcvbn/native` has). It is a measurement.

## What it does

`fdrace <threads> <per-thread> <long-every> <long-ms>` runs `<threads>` threads, each spawning `<per-thread>` `/bin/sh -c` children
with stdin null and stdout and stderr piped, as nextest spawns a test. One child in `<long-every>` lives `<long-ms>` ms (the possible
holder); the others `exit 0` at once. For each short child it waits for the exit, then times how long its stdout takes to reach EOF.
Nothing is left to hold that pipe but another process, so a delay of 100 ms or more is a pipe that something else inherited.

## Why macOS

On Linux std makes a pipe with `pipe2(O_CLOEXEC)`, one atomic step. macOS has no `pipe2`: std calls `pipe()` and then
`fcntl(FD_CLOEXEC)`. A child spawned by another thread between the two calls gets the write end without the flag and keeps it across
`exec` for as long as it lives, so the pipe it was not made for stays open until that child exits.

## Running it

```sh
cargo build --release --manifest-path tools/fd-race/Cargo.toml
tools/fd-race/target/release/fdrace 16 1500 8 400          # about 90 s; 20,992 short children

# the Linux control: the same program in a container
docker run --rm -v "$PWD/tools/fd-race:/w:ro" rust:1 sh -c \
  'cp -r /w /src && cd /src && rm -rf target && cargo build --release -q && /src/target/release/fdrace 16 1500 8 400'
```

## Result (2026-11-15)

`results/2026-11-15.txt` has every run. Five runs of `fdrace 16 1500 8 400`:

| Platform | Runs | Short children | Stdout EOF held 100 ms or more after exit | Worst |
|---|---|---|---|---|
| macOS 27.3, Rust 1.99.0 | 3 | 62,976 | **5** (1, 4, 0) | 408 ms |
| Linux 7.2 (OrbStack, aarch64), Rust 1.99.1 | 2 | 41,984 | **0** | 0 ms |

Each delay is about the 400 ms a holder lives. At the macOS rate, Linux would have shown about 3 in 41,984; it showed none.

## What it does not show

That nextest's own spawn path is the one that is hit in a `cargo nextest run` (the leak that was seen was not caught with `lsof`),
or what rate to expect there: 1 flagged test in 128,560 test runs (30 runs of the 3,922 hermetic tests and 20 of the 545 of
`thrust-api-v1`) on the one Mac that measured it.
