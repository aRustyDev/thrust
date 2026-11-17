# App and end-to-end

Status: **live** (Playwright journeys over the stock React app, `e2e/`) · **template-only** (axum router tests, Dioxus
SSR, in-process harness) · **planned** (desktop, mobile) · Gate: the `worker-cf e2e` and `worker-cf e2e-mail` cells of the contract matrix (also run daily by `e2e.yml`, which gates nothing)

## Catches

X18 (P: the stock app's journeys now; Dioxus SSR once a UI crate exists), X2 and X20 (B, router tests). Tests the
application layers above the library: a route's request/response mapping without a socket, a component's rendered
output, a whole flow through an in-process server, a user journey in a real browser.

## Use when / Not when

| Layer | Use | Test with |
|---|---|---|
| Logic extracted from a handler or component | always first | [example](example-and-parameterized.md) or [property](property.md) tests |
| One route, one request | status codes, extractors, error mapping | `tower::ServiceExt::oneshot` on the `thrust-api` router |
| A Dioxus component | rendered markup and state | `dioxus-ssr`, selecting by `data-testid` |
| A flow across the server and a web build | the wiring | an in-process server on `127.0.0.1:0` (package `e2e`, prefix `e2e_`) |
| A user journey in a real browser, on the stock React app | what only a browser shows: the screens, the network log | a Playwright spec in `e2e/journeys/`, run by `scripts/e2e.sh` (TypeScript, not a crate; ADR-0123) |
| A desktop or mobile shell | | not covered yet |

Not for HTTP behaviour that must hold on every adapter: that is the [contract suite](contract-suite.md).

## Name and location

Router and component tests in the crate they test. In-process e2e in a package named `e2e`, tests prefixed `e2e_`;
neither the package nor the `e2e` nextest profile exists yet (`.config/nextest.toml` says to add them from the
template when the crate exists). The browser journeys are a different thing with the same word: the TypeScript package
`e2e/` (Playwright, pinned by `e2e/pnpm-lock.yaml`), **one spec per journey** in `e2e/journeys/`, what they share in
`e2e/support/`. Thrust runs on the Cloudflare set only (Q-0123.2), and every journey is also run against the pinned Plane, the
UI oracle: green on Plane and red on Thrust is a parity defect, red on Plane means the journey is wrong. They are never part
of the default nextest loop (`AGENTS.md`, testing rule 6).

## Run

```sh
scripts/rs cargo nextest run -p thrust-api      # no tests yet [derived]
scripts/e2e.sh worker-cf controls               # the journeys, then their controls; needs Docker, node and pnpm
scripts/e2e.sh worker-cf mail controls          # the journeys that need a mail service (e2e/journeys-mail/), then theirs
scripts/e2e.sh running                          # against a Worker that is already up (THRUST_BASE_URL, THRUST_ADMIN_TOKEN)
ORACLE_DIR=<fork checkout>/oracle scripts/e2e.sh plane-oracle controls    # the pinned Plane; local only
E2E_RECORD_HAR=1 ORACLE_DIR=... scripts/e2e.sh plane-oracle               # and write the scrubbed HAR into e2e/har/
```

`scripts/e2e.sh` builds `web/` if it is not built, starts the target as `scripts/contract.sh` does (its `e2e` pass), and
ends in `VERDICT: PASS` or `VERDICT: FAIL`. The same run is the `worker-cf e2e` cell of the contract matrix.

`plane-oracle` is **local only**: the oracle builds from the fork checkout, which is not pushed (Q-0122.4). It builds the
pin's stock app (Thrust's `web/` with the Q-R2 ledger reverted to the pin, each file checked against `web/PIN-MANIFEST`),
brings the oracle stack up fresh beside it on one origin, does the instance-admin setup, runs the journeys and takes the stack
down. `down -v` drops the seeded snapshot that `scripts/oracle-v1.sh plane` needs, so re-seed after it. What it needs and
what it leaves out: `tools/e2e-plane/README.md`.

### The mail suite

Some screens need a target that has a mail service: the code step of the sign-in (D-042) is drawn only when `/api/instances/` says
`is_smtp_configured`, and what it is given is a code that was mailed. The first suite cannot hold such a journey, for three reasons:
`m2-sign-in-methods` asserts that the target has no mail (no "Sign in with unique code", the "ask your admin" line, no call of the magic
routes), the pinned Plane has none either, and the count guard holds every file of `e2e/journeys/` to "passed" on every run, so a journey
that needs mail would fail every run of the suite without it. So the journeys that need mail are a second suite, `e2e/journeys-mail/`,
run by its own pass (`scripts/contract.sh <target> e2e-mail`, the `worker-cf e2e-mail` cell, `scripts/e2e.sh <target> mail`) against a
Worker with the `mail` pass's mail service: the `send_email` binding as `wrangler dev` simulates it, which sends nothing and writes each
message to the server's log. `e2e/support/mail.ts` reads that log (`THRUST_MAIL_WRANGLER_LOG`, which `contract.sh` exports and the
browser run inherits): it strips wrangler's colours, finds the messages, and `waitForMail` reads again every 100 ms until the one the
journey needs is there, because the simulator's lines reach the log a few milliseconds after the response that caused them. It is the
small cousin of `simulated_mails` in `crates/testing/suites/api/tests/mail.rs`, and the code is the last word of the subject.

What the suite does not have, and why: the native targets (the native server does not serve the app), the pinned Plane (it writes its
code to its Valkey, and its stack has no mail sink; status.md G-44), and a committed HAR (it would hold a live code, so
`E2E_RECORD_HAR` is refused for it). `assertMailTarget` (`e2e/support/target.ts`) makes a journey aimed at a target without mail fail
at `TARGET GUARD (mail)`, and `contract.sh`'s vacuity guard refuses to start the pass on a Worker that does not say it has mail.

## Required controls

[Broken implementation](../evidence-and-controls.md#broken-implementation); for markup, assert on a `data-testid`
and a value, not on a class name that a broken render could still contain.

A journey that passes proves nothing until its detectors have been shown to fire, so `scripts/e2e.sh controls` runs the
journey in ways that must fail, and requires the failure **and** the words it fails with: a 404 the browser is shown
(`stickies-404`), a work-item list the browser empties (`empty-list`, must fail at "see it in the list"), two sign-in providers
the browser turns on (`sso-on`, the sign-in journey's own detector: it must fail at "a sign-in method other than the
password"), a change of password the browser answers as refused (`change-refused`: the password journey must fail at "change the
password") and one it answers as done and never sends (`change-dropped`: the app says "Password changed successfully." and the
server has changed nothing, so the journey must fail at "the other browser is signed out", which is the detector that makes the
journey look at the server's state and not at the toast or the 200 in its own network log), a journey that
skips itself (`skip`; Playwright reports a skip as a pass, so the **count guard** must refuse it), and a journey told it is
on the other target (must fail at the `TARGET GUARD`; a Thrust told it is on Plane, and Plane told it is a Thrust). One run is
the other way round and must **pass**: `slow-page` slows the browser to a CI runner's pace and holds the profile answer the home
page asks for, which the app applies over its own record of the answered welcome tour (the cell failed at "create a project"
like that, G-31); a journey that answers the tour once and goes on fails it, and one that looks that the tour stayed answered,
and answers it again if not, does not. It is only worth having because it was shown to fail the journey without that; a first
version of the journey that only waited for the page to go quiet passed it on a fast machine and failed it in CI. The count
guard holds every run's report to one passed journey per file in `e2e/journeys/` (`e2e/journeys-mail/` for the mail suite). A new
journey is a new file, and brings no control of its own unless it has a detector the others lack. The mail suite's detectors are
three: the browser tells the app that no mail service is configured (`smtp-off`: it must fail at "ask for a code", so the journey
looks for the button and does not reach the code step some other way), the browser changes the code in the form it posts to one that
no code is (`code-dropped`: the real server refuses it, and the journey must fail at "the code signs in", which makes it look at the
page after the post and not at the post), and the journey asks the log for the mail of an address that was never mailed
(`mail-unread`: it must fail at "read the code from the mail"; a reader that found some message whatever it was asked for would
hand over a code that is not the person's). `wrong-target`, `slow-page` and `skip` run on it as on the first suite. The HAR that is committed (`e2e/har/`) comes from a passing run
only, and `tools/e2e-har/scrub.py --self-test` shows its scrub removes planted secrets and that its check can fail.

## Owning skill

`.agents/skills/rust-test-app/SKILL.md` for the layers above a Rust library; it does not cover the Playwright journeys,
which have no skill of their own (this page is their procedure). Dioxus application code is `dioxus-07`; visual
comparison of a ported UI is `clone-frontend` (`.agents/skills/clone-frontend/scripts/pixeldiff.py`).

## In-repo examples

`e2e/journeys/m1-sign-up-to-work-item.spec.ts` (sign up to a listed work item), with the target adapter, network-log judge
and controls in `e2e/support/`; `e2e/journeys-mail/m2-sign-in-by-code.spec.ts` (a code is mailed, a wrong one and a replaced one are
refused, the right one signs in), with the mail reader in `e2e/support/mail.ts`. For the Rust layers, none yet; templates are under
`.agents/skills/rust-test-app/references/`.

## Target

The UI parity smoke set: ADR-0123 and the parity matrix in `.agents/plans/contrail/TRACEABILITY.md`, which tables each
journey with its screens and where it passes. The journeys for the M2 screens are issue #46's; each is a new file in
`e2e/journeys/`, passes on the pinned Plane and on `worker-cf`, and is a new row of that table. (A journey that needs a mail service is a
file of `e2e/journeys-mail/` and passes on `worker-cf` only, for now: G-44.)
