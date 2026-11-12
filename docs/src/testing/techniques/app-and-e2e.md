# App and end-to-end

Status: **live** (Playwright journeys over the stock React app, `e2e/`) · **template-only** (axum router tests, Dioxus
SSR, in-process harness) · **planned** (desktop, mobile) · Gate: the `worker-cf e2e` cell of the contract matrix (also run daily by `e2e-schedule.yml`, which gates nothing)

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

## Required controls

[Broken implementation](../evidence-and-controls.md#broken-implementation); for markup, assert on a `data-testid`
and a value, not on a class name that a broken render could still contain.

A journey that passes proves nothing until its detectors have been shown to fire, so `scripts/e2e.sh controls` runs the
journey in ways that must fail, and requires the failure **and** the words it fails with: a 404 the browser is shown
(`stickies-404`), a work-item list the browser empties (`empty-list`, must fail at "see it in the list"), a journey that
skips itself (`skip`; Playwright reports a skip as a pass, so the **count guard** must refuse it), and a journey told it is
on the other target (must fail at the `TARGET GUARD`; a Thrust told it is on Plane, and Plane told it is a Thrust). One run is
the other way round and must **pass**: `slow-page` slows the browser to a CI runner's pace and holds the profile answer the home
page asks for, which the app applies over its own record of the answered welcome tour (the cell failed at "create a project"
like that, G-31); a journey that answers the tour the moment it shows fails it, and one that lets the page settle first does
not. It is only worth having because it was shown to fail the journey without that wait. The count
guard holds every run's report to one passed journey per file in `e2e/journeys/`. A new journey is a new file, and brings no
control of its own unless it has a detector the others lack. The HAR that is committed (`e2e/har/`) comes from a passing run
only, and `tools/e2e-har/scrub.py --self-test` shows its scrub removes planted secrets and that its check can fail.

## Owning skill

`.agents/skills/rust-test-app/SKILL.md` for the layers above a Rust library; it does not cover the Playwright journeys,
which have no skill of their own (this page is their procedure). Dioxus application code is `dioxus-07`; visual
comparison of a ported UI is `clone-frontend` (`.agents/skills/clone-frontend/scripts/pixeldiff.py`).

## In-repo examples

`e2e/journeys/m1-sign-up-to-work-item.spec.ts` (sign up to a listed work item), with the target adapter, network-log judge
and controls in `e2e/support/`. For the Rust layers, none yet; templates are under
`.agents/skills/rust-test-app/references/`.

## Target

The UI parity smoke set: ADR-0123 and the parity matrix in `.agents/plans/contrail/TRACEABILITY.md`, which tables each
journey with its screens and where it passes. The journeys for the M2 screens are issue #46's; each is a new file in
`e2e/journeys/`, passes on the pinned Plane and on `worker-cf`, and is a new row of that table.
