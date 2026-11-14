# Research 0015 — What a zxcvbn dictionary costs the Worker (D-023)

Date: 2026-11-14 · Issue [#44](https://github.com/aRustyDev/thrust/issues/44) · Deviation D-023 · Base `77ec027` (main)
Status: **done for bundle size and startup; the first-call CPU is a local proxy only, not settled on the platform (section 4).**

Evidence tags: **[re-run]** I ran it here, with the command given · **[derived]** read from source, not run · **[not measured]**.

## 0. Read this first

1. **The reason D-023 gave, bundle size, does not hold.** Adding `zxcvbn` 3.1.1 with Plane's rule takes the Worker from **4.756 MiB to
   6.765 MiB (+2.009 MiB, +42.2%)**, 33.8% of ADR-0090's 20 MiB reopen trigger and a tenth of the platform's 64 MiB. **[re-run]**
2. **The dictionary is the minority of that.** Its text is 792,697 B (0.756 MiB, 93,855 entries); the other 1.25 MiB is code, not decomposed
   (`twiggy` and `cargo-bloat` are not installed). **[derived]** from the crate source for the dictionary; the rest by subtraction.
3. **Startup is unchanged**: a local profile window of 13 to 15 ms before and after, against the 300 ms target. **[re-run]**
4. **The first strength check in an isolate costs about 30 ms on local workerd** (8 ms native), against warm checks of about 2.4 ms.
   Bar 3 (no more than one argon2 hash, about 45 ms p50 deployed, `research/0012`) is **not settled**: Stage B (a deployed `cpuTime`
   read) was offered and the owner chose not to run it now.
5. **What does count against the crate is not size.** `scripts/check-deps.sh` forbids `wasm-bindgen`, `js-sys` and `web-sys` in `thrust-core`'s
   graph, and zxcvbn declares all three as `wasm32` dependencies. Tried, the guard fails with **15 hits across five crates** (`thrust-core`,
   `thrust-api`, `thrust-api-v1`, `thrust-compat-plane` and `thrust-plane-types`, each for all three), and passes with **0** on the unmodified base. Adopting the crate needs a port and an adapter crate, as
   `thrust-adapters-oidc` has. **[re-run]**

## 1. Bundle size

Build: `cd crates/thrust-worker && ../../scripts/rs worker-build --release . --no-default-features` (the command at `deploy.yml:190`).
Sizes by `wc -c` and `gzip -9` of `build/index_bg.wasm`, `build/index.js`, `build/worker/shim.mjs` (spike 0001's method, `research/0004` 6.1).

| Build | `index_bg.wasm` | total (3 files) | gzip -9 | dictionary text in the wasm |
|---|---|---|---|---|
| Base, built twice (identical bytes) | 4,950,404 | 4,987,284 (4.756 MiB) | 1,560,996 | absent |
| Control: `zxcvbn` referenced by an uncalled `pub fn` | 4,953,037 | 4,989,917 (4.759 MiB) | 1,561,701 | absent |
| Plane's rule called in `password_is_strong` | 7,056,943 | 7,094,031 (6.765 MiB) | 2,386,414 | present, once each |
| Same, `default-features = false` | 7,052,467 | 7,089,555 (6.761 MiB) | 2,385,628 | present |

`wrangler check startup` reports the bundle as 4895.81 KiB (gzip 1534.11) for the base and 6953.40 KiB (gzip 2341.54) with the call.
The spike's 1.55 MiB was stale: the Worker had grown to 4.76 MiB before this change, so no old figure should be reused.

**The control did not come out byte-equal.** It is +2,633 B (+0.053%) over the base. Neither dictionary needle nor the crate's source path
is in it and `index.js` is identical, so no zxcvbn code or data was linked; the cause of the 2.6 KB is not established (the lockfile gained
only new packages). Read it as the noise floor: it is 0.1% of the delta.

The needles are a run of 12 consecutive entries from the middle of the password list and of the English Wikipedia list, cut from the crate source.

## 2. What was changed to measure it (throwaway, never committed)

`crates/thrust-core/Cargo.toml`: `zxcvbn = "=3.1.1"` (or with `default-features = false`). In `password_is_strong`, after the existing checks:

```rust
let cs: Vec<char> = password.chars().collect();
let first: String = cs.iter().take(72).collect();
let last: String = cs.iter().rev().take(72).rev().collect();
zxcvbn::zxcvbn(&first, &[]).score() >= zxcvbn::Score::Three && zxcvbn::zxcvbn(&last, &[]).score() >= zxcvbn::Score::Three
```

`custom_wasm_env` was not used: it adds a wasm import module named `zxcvbn` that `worker-build`'s glue does not supply. **[derived]**

## 3. Startup

`scripts/startup-check.sh wrangler.toml`, five runs each, the bundle size printed per run to prove which build was profiled:
base 13.2 to 13.5 ms; with the call 13.5 to 15.5 ms (ten runs). The first run of each was a cold-cache outlier (18.7 and 22.5 ms) and is not
used. The dictionaries are built lazily (`lazy_static`), so nothing runs at global scope. **[re-run]**

## 4. The first call

The six dictionaries and the matchers' regexes are built on the first `zxcvbn()` call in an isolate.

| Where | First strength check | Warm | Evidence |
|---|---|---|---|
| Native release, 20 cold processes | 7.7 to 9.4 ms | about 0.6 ms (two `zxcvbn` calls); 3.7 to 4.4 ms for a 72-character password | **[re-run]**, a lower bound |
| Local workerd, 5 cold starts, `POST /internal/operator/users` | 31.2 to 32.8 ms (wall, loopback) | 2.0 to 3.0 ms (15 samples, mean 2.4) | **[re-run]**, wall clock, not CPU |
| Cloudflare, `cpuTime` | | | **[not measured]** |

A request refused before zxcvbn runs (too short) takes about 6 ms on the same Worker, so the surcharge is about 30 ms. Retained heap after the
first call is **4,854,543 B native** (peak 5,172,648 B), against a 128 MB isolate. On wasm32 a `&str` is 8 bytes, not 16, so the Worker's figure should be smaller **[derived]**,
not measured.

Controls: `Password123!` scores One and the passphrase scores Four; `abcdefghijkl` (Zero) and `qazwsxedcrfv` (One) are accepted by the old
rule and refused by zxcvbn, so on workerd a 400 for them can only come from zxcvbn, and the passphrase got 204. Local workerd answered 401
without the token and 409 for a repeated email. The port check had a positive control (`lsof` saw two listeners while the server was up, none after the kill).

## 5. What this does not settle

- **The platform CPU of the first call** (bar 3). Stage B reuses `scripts/measure-argon2.sh` with a weak password, which is refused before any hashing.
- **Whether the Rust crate scores as Python's zxcvbn does** (Plane's), and so whether it refuses the same passwords. **[not measured]**
- **The crate's panic exposure** under ADR-0006's rule that request data cannot panic. **[not measured]**
- Whether `check-deps.sh` should carve out a strength-check adapter crate: a design question for whoever reopens D-023.

## 6. Reproduce

Base `77ec027`; macOS arm64; rustc 1.99.0; `worker-build` 0.8.7 (CI's pin); wrangler 4.123.0 locally (CI pins 4.151.2); `zxcvbn` 3.1.1.
`wrangler check startup` re-runs the `[build]` command here, so it profiles the current source, not whatever is in `build/` (spike 0001 said it
reuses it; the first "baseline" repeat in this work profiled the wrong build until the printed bundle size showed it).
