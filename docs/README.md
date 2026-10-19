# thrust docs

The book is an mdBook. Build it from the repo root with `mdbook build docs` and open `docs/book/index.html`, or run
`mdbook serve docs`. [AGENTS.md](AGENTS.md) has the prerequisites, the checks and the rules for writing a page.

## What it covers

| Section | Pages |
|---|---|
| Architecture | `src/architecture/app.md` (crates, ports, composition roots), `src/architecture/infra.md` (Worker, server, Postgres) |
| Flows | `src/flows/app.md` (the HTTP surface, permissions, the write path, errors) |
| Testing | `src/testing/` (strategy, the TDD loop, techniques, rules, status) |

## Not written yet

These have no page because there is nothing in the repository to describe yet, or the choice is not made:

| Page | Why |
|---|---|
| CI/CD | There are no CI workflows. ADR-0100, ADR-0101 and ADR-0110 are decisions only |
| Justfile recipe flow | There is no justfile; the entry points are `scripts/*.sh` |
| User flows | No user-flow document exists. `.agents/plans/contrail/TRACEABILITY.md` leaves the P0 journey set as TBD, to be defined during spike 0009 |
| Contributing: crates, Cargo features, standards, patterns | Not written yet |
| Contributing: code of conduct | Choosing one is a project decision |
| Contributing: fixes | What it should cover is undecided |
| API testing with Hurl, xh or httpie | None of them is used or installed; the contract suite is Rust |
| Spec | Nothing has graduated from the plan (`.agents/plans/contrail/SPEC.md` is a draft) |
