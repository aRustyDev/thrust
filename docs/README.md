# thrust docs

The book is an mdBook. Build it from the repo root with `mdbook build docs` and open `docs/book/index.html`, or run
`mdbook serve docs`. [AGENTS.md](AGENTS.md) has the prerequisites, the checks and the rules for writing a page.

## What it covers

| Section | Pages |
|---|---|
| Architecture | `src/architecture/app.md` (crates, ports, composition roots), `src/architecture/infra.md` (Worker, server, Postgres), `src/architecture/cicd.md` (the CI pipeline and its one required check) |
| Flows | `src/flows/app.md` (the HTTP surface, permissions, the write path, errors) |
| UI/UX | `src/uiux/` (the draft earth-pigment palette, its contrast and colour-blindness checks, and how the book is themed from `.config/branding/`) |
| Contributing | `src/contributing/` (crates, Cargo features, standards and their gates, patterns) |
| Testing | `src/testing/` (strategy, the TDD loop, techniques, rules, status) |

## Not written yet

These have no page because there is nothing in the repository to describe yet, or the choice is not made:

| Page | Why |
|---|---|
| Justfile recipe flow | There is no justfile; the entry points are `scripts/*.sh` |
| User flows | No user-flow document exists. `.agents/plans/contrail/TRACEABILITY.md` leaves the P0 journey set as TBD, to be defined during spike 0009 |
| Contributing: code of conduct | Choosing one is a project decision |
| Contributing: fixes | No source says what it should cover |
| API testing with Hurl, xh or httpie | None of them is used or installed; the contract suite is Rust |
| Spec | Nothing has graduated from the plan (`.agents/plans/contrail/SPEC.md` is a draft) |
