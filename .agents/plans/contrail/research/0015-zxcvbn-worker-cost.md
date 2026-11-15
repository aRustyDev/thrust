# Research 0015 — What a zxcvbn dictionary costs the Worker (D-023)

Date: 2026-11-14 · Issue [#44](https://github.com/aRustyDev/thrust/issues/44) · Deviation D-023 · Base `77ec027` (sections 1 to 4), `990e1ce` (section 4.2)
Status: **done. Bundle size, startup and the first call's CPU on Cloudflare are measured. Bar 3 (first-call CPU) is missed at the median by
about 15 ms (section 4.2); bars 1 and 2 are met. The owner reopened D-023 on 2026-11-14 to take that measurement and, having seen the figure and
how rarely a password is set (section 4.3), decided to leave the rule as it is. The harness, tests, results and analysis are in
`tools/measure-zxcvbn/` (section 6).**

Evidence tags: **[re-run]** I ran it here, with the command given · **[tested-cf]** run against Workers deployed on Cloudflare's network
(the Woven account, as `research/0012`) · **[derived]** read from source, not run · **[not measured]**.

## 0. Read this first

1. **The reason D-023 gave, bundle size, does not hold.** Adding `zxcvbn` 3.1.1 with Plane's rule takes the Worker from **4.756 MiB to
   6.765 MiB (+2.009 MiB, +42.2%)**, 33.8% of ADR-0090's 20 MiB reopen trigger and a tenth of the platform's 64 MiB. **[re-run]**
2. **The dictionary is the minority of that.** Its text is 792,697 B (0.756 MiB, 93,855 entries); the other 1.25 MiB is code, not decomposed
   (`twiggy` and `cargo-bloat` are not installed). **[derived]** from the crate source for the dictionary; the rest by subtraction.
3. **Startup is unchanged**: a local profile window of 13 to 15 ms before and after, against the 300 ms target. **[re-run]**
4. **The first zxcvbn call in a fresh isolate costs about 60 ms of CPU on Cloudflare** (median 60.5 ms, 95% bootstrap interval 54.5 to 64.0),
   on top of a cold start of about 25 ms that every route pays. That misses bar 3 (no more than one argon2 hash, about 45 ms p50 deployed,
   `research/0012`) by about 15 ms at the median; a cold isolate's whole first request is a median 85 ms and a p90 of 146 ms. A warm check
   is 1 to 2 ms. Locally it was 30 ms on workerd and 8 ms native, so the local figures understated the platform by a factor of 2 to 8. **[tested-cf]**
4a. **The 2 MiB bigger module costs a cold start nothing measurable**: baseline and variant cold starts have medians of 26 and 25 ms
   (difference -1 ms, interval -9 to 5). **[tested-cf]**
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
| Cloudflare, `cpuTime`, 18 fresh Workers (section 4.2) | median 60.5 ms above a cold start (interval 54.5 to 64.0) | 1 to 2 ms | **[tested-cf]**, the platform's CPU |

A request refused before zxcvbn runs (too short) takes about 6 ms on the same Worker, so the surcharge is about 30 ms. Retained heap after the
first call is **4,854,543 B native** (peak 5,172,648 B), against a 128 MB isolate. On wasm32 a `&str` is 8 bytes, not 16, so the Worker's figure should be smaller **[derived]**,
not measured.

Controls: `Password123!` scores One and the passphrase scores Four; `abcdefghijkl` (Zero) and `qazwsxedcrfv` (One) are accepted by the old
rule and refused by zxcvbn, so on workerd a 400 for them can only come from zxcvbn, and the passphrase got 204. Local workerd answered 401
without the token and 409 for a repeated email. The port check had a positive control (`lsof` saw two listeners while the server was up, none after the kill).

### 4.2 On Cloudflare (the owner's request of 2026-11-14: "reopen D-023 to check the Cloudflare cpuTime of the first strength check")

**Method.** The #146 harness (`research/0012`): `wrangler tail` does not print `cpuTime`, so a tail consumer writes each invocation's
`cpuTime` to KV, and CPU is measured only on Cloudflare's network. The memory adapter set, no bindings, built twice from `990e1ce`: the
**baseline**, and the **variant** with Plane's rule (section 2). Two requests to `POST /internal/operator/users`, both refused with 400
before any hashing, so no argon2 is in the figure: **S**, the password `short` (refused by the length rule; zxcvbn never runs), and **Z**,
`abcdefghijkl` (accepted by the old rule, refused by zxcvbn with score Zero; the dictionaries are built). Three arms: **bS** (baseline, S),
**vS** (variant, S), **vZ** (variant, Z), six rounds each, rotated so no arm is always first. Each round is a **new Worker**
(`thrust-spike-zxcvbn-rNN`), a probe of the route until it answers 400 five times running, five requests on one connection, then two
bursts of 16 concurrent requests: 666 requests in all, every one matched to its platform record. The Workers, the tail consumer and the
KV namespace were deleted by trap and checked afterwards: all 19 Workers answer "does not exist" (code 10007) and the account has
the one KV namespace it had before.

**Cold or warm is read from the cost, not assumed.** A request on a fresh isolate costs the cold start (the S arms) and, on vZ, the
dictionaries too; a warm request costs 1 to 4 ms in every arm. So a request above 10 ms is a cold one, and the result is the same at
5, 10 and 15 ms.

| Arm | cold requests | p25 | p50 | p75 | p90 | max | warm requests (p50, mean) |
|---|---|---|---|---|---|---|---|
| bS (baseline, S) | 61 of 222 | 24 | 26 | 41 | 44 | 58 | 1 ms, 1.26 |
| vS (variant, S) | 49 of 222 | 23 | 25 | 39 | 47 | 67 | 1 ms, 1.36 |
| vZ (variant, Z) | 94 of 222 | 74 | 85.5 | 103 | 146 | 274 | 2 ms, 1.80 |

- **The first zxcvbn call in a fresh isolate** = median(vZ cold) - median(S cold) = 85.5 - 25.0 = **60.5 ms**, 95% bootstrap interval 54.5 to
  64.0 (request-level resampling, so a little narrower than it should be). Against the variant's own S arm alone: 60.5, 53.5 to 65.0.
- **Rounds as the independent units agree**: the six vZ rounds' cold medians are 76.5 to 91 ms and every S round's is 41 or less; all six
  vZ rounds lie above all twelve S rounds.
- **The bigger module costs a cold start nothing measurable**: cold medians 26 (baseline) and 25 (variant), difference -1 ms, interval -9 to 5.
- **A warm check is cheap**: 1 to 2 ms at the median, 0.5 ms above the S arms on average.
- **A burst makes it worse.** vZ had 94 cold requests of 222 and the S arms 49 and 61: a first call blocks its isolate for about 85 ms, so
  concurrent requests go to new isolates, each of which builds the dictionaries again. After a deploy or a scale-out, concurrent sign-ups
  pay the first call once per isolate, not once.

**One control did not behave as I pre-registered, and it limits what the same-connection sequences can say.** I expected the S arms'
"first request minus later requests" on one connection to be about 0 once the probe had absorbed the route's first-request overhead. It
was 0 to 45 ms (bS: 21, 45, 0, 26, 40, 25; vS: 0.5, 0, 0, 21, 26, 37.5): the sequence's connection is new, and the platform sometimes puts
it on a fresh isolate, which pays a cold start that is not zxcvbn. So vZ's first-minus-later (median 75.8, 37 to 145) contains a cold start
and is not reported as zxcvbn's. The 60.5 ms comes from the cold-cluster comparison above, in which both sides pay the cold start.

**Pilots that shaped the design.** A first pilot redeployed one Worker per round and tagged each version: 23 of 111 responses came
from the previous version's isolates seconds after the deploy, so builds were mixed; a new Worker per round has no previous version.
It also treated the second burst as warm. It is only partly warm: in the full run 12 to 27% of the second burst's requests were still cold, against 35 to
64% of the first's (`cold-analysis.txt`), because concurrent requests keep reaching fresh isolates.

**Caveats.** One account, one colo (the one this Mac's traffic reached; not recorded), one session on 2026-11-14; edge CPUs differ. `cpuTime` has a
resolution of 1 ms. Z measured one 12-character password: the first call's cost is the dictionaries and the matchers' regexes, which do
not depend on the input, but the cost of matching a long password on the platform is not measured (native: 3.7 to 4.4 ms for 72
characters). The Workers were the memory adapter set, not the Cloudflare set, which changes nothing the strength check does.

### 4.3 How often it runs (the owner's question of 2026-11-14; **[derived]** from the code: there is no usage data to measure)

The strength check runs when a password is **set**, not when someone signs in. `thrust_core::password_is_strong` has five call sites, reached from six entry points:

- **sign-up**: the compat route (`thrust-compat-plane/src/auth.rs`, S05, in Plane's order, after the email-exists lookup) **and** `create_account` inside
  `thrust_core::sign_up`, so **twice per sign-up**;
- `change_password` (Profile -> Security);
- `reset_password` (the reset by email, #169, dark until a mail service is wired);
- the operator's routes, off unless switched on: `admin_create_user` (through `create_account`) and `admin_set_password`.

`sign_in`, sessions, API keys and page loads do not call it, and a user who signs in through SSO never sets a password here. Per user that is once at account creation and
then a few times a year: per active user per day, close to zero.

Because calls are that rare, a password-setting request will usually land on an isolate that has not built the dictionaries (reasoning, not measured), so the 60 ms is
the typical cost of a call and the 1 to 2 ms warm figure applies only to bursts. In total CPU it is small: 1,000 active users each setting a password about 3 times a year are
about 3 minutes of CPU a year (illustrative arithmetic).

The exception is unauthenticated sign-up. Gap G-16 (`docs/src/testing/status.md`) records that Thrust does not throttle it (the Plane pin allows 10 a minute per IP), and the
check runs before any hashing, so a caller can reach it at any rate. With the current rule that costs microseconds. With zxcvbn it would cost 1 to 2 ms warm (measured on the
operator route, which runs one check; the compat route runs two, so about double, **[derived]**), and about 60 ms on each fresh isolate a flood reaches: 94 of 222 variant-Z requests
were cold in the full run, against 49 and 61 of 222 for the S arms.

## 5. What this does not settle

- **Bar 3 is a judgement, and the owner can move it.** 60.5 ms is 1.3 times one argon2 hash (about 45 ms p50). The platform's CPU limit
  is far above it (`research/0012` found the hash fits the platform "by a factor of 300 or more"), and a sign-up that passes the check
  pays the hash as well; the cost is latency and CPU on the first sign-up or password change in each isolate, multiplied by concurrency
  (section 4.2).
- **Whether the Rust crate scores as Python's zxcvbn does** (Plane's), and so whether it refuses the same passwords. **[not measured]**
- **The crate's panic exposure** under ADR-0006's rule that request data cannot panic. **[not measured]**
- Whether `check-deps.sh` should carve out a strength-check adapter crate: a design question for whoever reopens D-023.

## 6. Reproduce

Base `77ec027`; macOS arm64; rustc 1.99.0; `worker-build` 0.8.7 (CI's pin); wrangler 4.123.0 locally (CI pins 4.151.2); `zxcvbn` 3.1.1.
`wrangler check startup` re-runs the `[build]` command here, so it profiles the current source, not whatever is in `build/` (spike 0001 said it
reuses it; the first "baseline" repeat in this work profiled the wrong build until the printed bundle size showed it).

Section 4.2: base `990e1ce`, the Woven account, wrangler 4.123.0, curl 8.7.1 (`--variable` needs curl 8.3 and `%header{}` 7.84), 2026-11-14.
Both builds are `worker-build --release .` in `crates/thrust-worker` (the memory adapter set, which needs no bindings): the baseline is
5,449,824 B for `index_bg.wasm` (5.233 MiB with the glue) and the variant 7,554,406 B (7.240 MiB), a delta of 2.007 MiB, the same as on the
Cloudflare set (section 1).

**The harness, the tests, the results and the analysis are in `tools/measure-zxcvbn/`** (kept at the owner's request of 2026-11-14). Its README says how to run
each part, which test was shown to be able to fail and how, and which result files are generated and which transcribed from the terminal. The Cloudflare driver is the
pattern of `scripts/measure-argon2.sh` with a new Worker per round, the token in `--secrets-file` (a 0600 file), a preflight that refuses to run if any of its 19 Worker
names or the KV title exists (a `wrangler deploy` on an existing name overwrites that Worker), a refusal to run unless the baseline's wasm lacks the dictionary and the
variant's has it, and a teardown that deletes only what the run created. The variant is a patch (`patches/`), never committed as source: the `zxcvbn` crate must not enter
the workspace (section 0, item 5).
