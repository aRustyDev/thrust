# docs

The thrust book: an mdBook (`book.toml`, `src/SUMMARY.md`). It records conclusions, in the present tense, about
what the repository contains. Plans, ADRs and work in progress are in `.agents/plans/`, not here.

## Build and check

Needs mdBook 0.5.x and, for the diagrams, `mdbook-mermaid` on `PATH` (`cargo install` puts it in
`~/.cargo/bin`, which a shell may not have on `PATH`). Installing either is the user's decision.

```sh
mdbook build docs > build.log 2>&1   # from the repo root; the output goes to docs/book/ (git-ignored)
grep -E '^ERROR' build.log           # must print nothing: mdBook exits 0 on a broken include
```

`mdbook build` also exits 0 when a page is missing from `src/SUMMARY.md`; the page is silently left out. Run the
orphan check in `.agents/skills/mdbook/references/book-structure-and-config.md` and link every page. No CI job builds
the book, and a pull request that changes only `docs/` skips the Rust jobs, so these checks are yours to run. The
skill `mdbook` covers the rest (`mdbook test`, link checking, CI and Pages).

`mermaid.min.js` and `mermaid-init.js` in this directory are written by `mdbook-mermaid install docs`. Regenerate
them with that command and review the diff; do not edit them by hand.

## Writing a page

- Describe what exists. Anything planned goes under a "Not built yet" heading that names the plan file.
- Before naming a crate, feature, environment variable, route or script, find it in the tree. Re-run a command
  before you write what it prints.
- Cite repository files as root-relative code spans (`scripts/contract.sh`), not links: the book publishes only
  `docs/src/`, so a link to a file outside it breaks.
- Do not paste code. Name the file and the symbol, or include a region by anchor with `{{#include path:anchor}}`.
- Tag every code fence (`sh`, `toml`, `text`, `mermaid`), so `mdbook test` does not compile prose. Tag commands the
  way the testing chapter does: `[verified <date> <sha>]`, `[unverified: not run]` or `[derived]`.
- State a fact in one place and link to it from the others. The owner of each kind of fact is in
  `src/testing/README.md`, under "Where each fact lives".
- Create a page when it has real content. Do not add a stub to hold a place; `README.md` lists what is not written.
