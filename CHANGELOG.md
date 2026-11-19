# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/aRustyDev/thrust/compare/v0.2.0...v0.3.0) - 2026-11-19

### Added

- *(queue)* permanent versus transient consumer errors, and a dead_letters table on every target ([#310](https://github.com/aRustyDev/thrust/pull/310))
- *(issues)* clean a description once in the use-cases and store its text and policy ([#305](https://github.com/aRustyDev/thrust/pull/305))
- *(auth)* sign in with an emailed code, sign-in only (F-A04, D-042) ([#220](https://github.com/aRustyDev/thrust/pull/220))
- *(html)* make the hardened sanitiser the default, keeping the editor's embed and rel ([#302](https://github.com/aRustyDev/thrust/pull/302))
- *(edge)* say which build of the edge answered, and carry the edge in the attested deploy bundle ([#294](https://github.com/aRustyDev/thrust/pull/294))

### Fixed

- *(html)* read a style's CSS comments and escapes before judging it ([#309](https://github.com/aRustyDev/thrust/pull/309))
- *(compat-plane)* name the configured public address in /api/instances/, not the host the request came in on ([#233](https://github.com/aRustyDev/thrust/pull/233))

### Other

- *(core)* reach the asset model's situations by construction, not by a random draw ([#280](https://github.com/aRustyDev/thrust/pull/280))
- *(api-v1)* read a work item's request into a typed struct that writes its own schema ([#250](https://github.com/aRustyDev/thrust/pull/250))
- *(api-v1)* read a project's request into a typed struct that writes its own schema ([#247](https://github.com/aRustyDev/thrust/pull/247))
- *(api)* keep error DTOs at router boundary ([#263](https://github.com/aRustyDev/thrust/pull/263))
- *(html)* move the sanitiser into crates/core/html ([#300](https://github.com/aRustyDev/thrust/pull/300))
- move every crate into its ADR-0009 directory (crates-layout PR 3) ([#221](https://github.com/aRustyDev/thrust/pull/221))
- *(api-v1)* write a request struct, its reader and its schema from one line a field ([#261](https://github.com/aRustyDev/thrust/pull/261))
- *(api-v1)* read a label's request into a typed struct that writes its own schema ([#242](https://github.com/aRustyDev/thrust/pull/242))
- *(api-v1)* read a state's request into a typed struct that writes its own schema ([#235](https://github.com/aRustyDev/thrust/pull/235))
- *(api-v1)* hold each request schema to its parser with bodies the schema generates (request-parser step 0) ([#228](https://github.com/aRustyDev/thrust/pull/228))
- Guard preview migration transaction boundaries ([#322](https://github.com/aRustyDev/thrust/pull/322))
- Add transaction-scoped migration locking ([#315](https://github.com/aRustyDev/thrust/pull/315))
- Allow borrowed migration input ([#308](https://github.com/aRustyDev/thrust/pull/308))
- *(deps)* move the worker crates to workers-rs 0.9 ([#303](https://github.com/aRustyDev/thrust/pull/303))
- Add preview provisioning control plane ([#279](https://github.com/aRustyDev/thrust/pull/279))
- the load suite, thrust-load-tests, with the latency and scale probes as nextest tests (test-suites PR 3) ([#262](https://github.com/aRustyDev/thrust/pull/262))
- the v1 router's CPU profile, under samply through a nextest wrapper script (test-suites PR 5) ([#272](https://github.com/aRustyDev/thrust/pull/272))
- the profiling crate, thrust-profiling, with the v1 router's heap profile (test-suites PR 4) ([#267](https://github.com/aRustyDev/thrust/pull/267))
- update Cargo.lock dependencies

## [0.2.0](https://github.com/aRustyDev/thrust/compare/v0.1.0...v0.2.0) - 2026-11-16

### Added

- *(api-v1)* describe the Work Items operations in the OpenAPI document, which now has all 32 of the pin's operations ([#203](https://github.com/aRustyDev/thrust/pull/203))
- *(api-v1)* describe the Labels operations in the OpenAPI document, written from the type the routes answer with ([#193](https://github.com/aRustyDev/thrust/pull/193))
- *(api-v1)* describe the States operations in the OpenAPI document, written from the type the routes answer with ([#188](https://github.com/aRustyDev/thrust/pull/188))
- *(migrate)* apply each migration once, recorded in a ledger, and refuse a changed or contract file on request ([#201](https://github.com/aRustyDev/thrust/pull/201))
- *(worker)* send the password reset mail from the Worker through Cloudflare Email Sending (E3) ([#200](https://github.com/aRustyDev/thrust/pull/200))

### Other

- name every pinned regression regression_, so one filterset finds them all ([#209](https://github.com/aRustyDev/thrust/pull/209))
- crate paths that work at any directory depth (crates-layout PR 2) ([#212](https://github.com/aRustyDev/thrust/pull/212))
- *(api-v1)* every v1 route reads JSON only, to be revisited if a real client sends a form ([#214](https://github.com/aRustyDev/thrust/pull/214))

## [0.1.0](https://github.com/aRustyDev/thrust/releases/tag/v0.1.0) - 2026-11-15

### Added

- *(api-v1)* describe the Projects operations as an OpenAPI document and diff it against the pinned schema ([#179](https://github.com/aRustyDev/thrust/pull/179))
- *(auth)* the password reset by email, first half: the port, the token store and the routes, dark until a mail service is wired ([#169](https://github.com/aRustyDev/thrust/pull/169))
- *(auth)* serve the stock app's OIDC routes, dark until a provider is configured ([#163](https://github.com/aRustyDev/thrust/pull/163))
- *(auth)* let a person change their own password ([#151](https://github.com/aRustyDev/thrust/pull/151))
- *(auth)* let the operator make an account and set a password ([#149](https://github.com/aRustyDev/thrust/pull/149))
- *(compat)* the asset stage, S28, S29, S31 and S33 ([#124](https://github.com/aRustyDev/thrust/pull/124))
- *(api)* the oracle's seed and POST /internal/reset, for the v1 gate ([#104](https://github.com/aRustyDev/thrust/pull/104))
- *(api-v1)* the work items of /api/v1 (stacked on #99) ([#102](https://github.com/aRustyDev/thrust/pull/102))
- *(api)* /api/v1 projects, states, labels and work items, with the oracle gate on those operations ([#99](https://github.com/aRustyDev/thrust/pull/99))
- *(compat)* work item change and history: X03, X02, and the sub work items and relations the page asks for ([#97](https://github.com/aRustyDev/thrust/pull/97))
- *(compat)* work items, the list, the create and the detail: S47, S48, X01 ([#85](https://github.com/aRustyDev/thrust/pull/85))
- *(compat)* project create and open: S30, S32, S34 to S46 ([#84](https://github.com/aRustyDev/thrust/pull/84))
- *(compat)* workspace load and home: S17, S19 to S26 ([#83](https://github.com/aRustyDev/thrust/pull/83))
- *(compat)* the workspace half of boot and onboarding: S08, S09, S12 to S16, S18 ([#82](https://github.com/aRustyDev/thrust/pull/82))
- *(compat)* the profile half of boot and onboarding: S02 signed in, S07, S10, S11, S27 and the onboard route ([#81](https://github.com/aRustyDev/thrust/pull/81))
- *(compat)* the auth stage, S03 to S06: CSRF token, email-check, sign-up and sign-in as JSON ([#80](https://github.com/aRustyDev/thrust/pull/80))
- *(db)* engine pass, search routes and the FR-22 order; spike 0002 decides Postgres ([#77](https://github.com/aRustyDev/thrust/pull/77))
- *(auth)* version the API key hash and switch CSRF enforcement; record the #38 review decisions ([#78](https://github.com/aRustyDev/thrust/pull/78))
- *(auth)* the Rust auth slice: argon2id sign-in, cookie sessions with CSRF, API keys, one OIDC login ([#76](https://github.com/aRustyDev/thrust/pull/76))
- *(ports)* sessions are Database rows; delete SessionStore, CacheError and KvSessionStore
- *(core)* authorize resolves the workspace slug; Principal is user-only
- *(ports)* repository and Tx methods take a Scope or a SystemScope, never ids
- *(spike)* ports/core/contracts workspace, in-memory adapters, native server, shared HTTP suite

### Fixed

- *(api)* read a limited guest's project flag in the page statement, not before it ([#112](https://github.com/aRustyDev/thrust/pull/112))
- *(core)* reject "." as an attachment name

### Other

- *(contrail)* measure the first zxcvbn call's CPU on Cloudflare ([#171](https://github.com/aRustyDev/thrust/pull/171))
- *(contrail)* measure what the zxcvbn crate would cost the Worker ([#170](https://github.com/aRustyDev/thrust/pull/170))
- *(rate-limits)* delete the slice's create limit, its binding and its settings ([#166](https://github.com/aRustyDev/thrust/pull/166))
- *(adapters)* delete the port reads that only the slice's list and search used ([#164](https://github.com/aRustyDev/thrust/pull/164))
- *(core)* delete the work-item use-cases that only the slice's routes called ([#161](https://github.com/aRustyDev/thrust/pull/161))
- *(api)* delete the slice's sign-in, sign-out and CSRF-token routes ([#150](https://github.com/aRustyDev/thrust/pull/150))
- *(ci)* enforce the clippy deny list, rustfmt and cargo-deny; forbid ORM crates workspace-wide ([#87](https://github.com/aRustyDev/thrust/pull/87))
- format the workspace with rustfmt (max_width 120) ([#86](https://github.com/aRustyDev/thrust/pull/86))
- describe sessions as database rows and authorization by workspace slug as built
- *(core)* guard that only authorize mints a Scope and only the roots mint a SystemScope
- *(ports)* declare the Scope and SystemScope proof types in thrust-ports
- *(core)* generator coverage for the core model and validator properties
- *(core)* add example, property, model and outbox tests for thrust-core
