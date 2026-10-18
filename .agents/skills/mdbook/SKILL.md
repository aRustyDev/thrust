---
name: mdbook
description: Use when an mdBook (`book.toml`, `SUMMARY.md`) misbehaves or is being set up - a chapter is missing from the sidebar, `unknown field` after upgrading to mdBook 0.5, `mdbook test -L target/debug/deps` fails with `E0432 unresolved import` or `E0464 multiple candidates for rlib`, `{{#include file.rs:anchor}}` shows up literally in the page, `mdbook-mermaid` or `mdbook-linkcheck2` setup, deploying an mdBook to GitHub Pages. NOT for `cargo doc` / rustdoc pages, MkDocs, Docusaurus or Hugo sites, or generic Markdown editing.
allowed-tools:
  - Edit(docs/**)
  - Bash(mdbook --version)
author: aRustyDev
version: 0.1.0
license: AGPL-3.0-only
---

# mdBook 0.5.x

A green `mdbook build` proves little: it exits 0 on a broken `{{#include}}`, on a mistyped `[output.html]`
key and on chapters missing from `SUMMARY.md`. The log and the file list are the test.

The `Edit` rule covers file writes and pre-approves `docs/**` only (the book root is assumed to be `docs/`).
Only `mdbook --version` runs without a prompt: `mdbook build`, `mdbook test` and `cargo build` ask first.
Rust sources (anchors in `examples/*.rs`, a `#[cfg(doctest)]` item) and `.github/workflows/` are edited only
with the user's approval.
Installing a preprocessor or running `cargo install` is the user's decision: propose it and wait.

| Need | Read |
|---|---|
| `SUMMARY.md`, `book.toml`, 0.5 changes, silent failures, orphan check | `references/book-structure-and-config.md` |
| Compiling code blocks, `mdbook test -L`, sharing snippets with doc-tests | `references/code-blocks-and-doctests.md` |
| Mermaid, admonitions, link checking | `references/preprocessors-and-linkcheck.md` |
| CI and Pages (`references/examples/docs.yml`) | `references/ci-and-pages.md` |

## Procedure

1. `mdbook --version` must be 0.5.x. Set `[build] create-missing = false` and `[rust] edition` to match the
   linked crate's edition.
2. Link every chapter from `SUMMARY.md`.
3. Share code, never paste it: anchor a region of a cargo-compiled file and `{{#include path:anchor}}` it in a
   `rust,ignore` block.
4. Build into a log; fail on any line starting `ERROR`; run the orphan check.
5. `cargo build` the linked crate into a dedicated `CARGO_TARGET_DIR`, then `mdbook test <dir> -L <that>/debug/deps`.
6. Publish with a pinned mdBook version and least-privilege Pages permissions.

## Output contract

Build log with zero `ERROR` lines, empty orphan list, `mdbook test` exit 0, tool versions. Tags:
`[unverified: not run]` = command not executed; `[derived]` = taken from documentation, not executed.

## Example

Q: "CI is green but the Deployment page is not in the sidebar." A: the file is not linked from `SUMMARY.md`
and mdBook omits it silently. Run the orphan check, link the page, add the `^ERROR` and orphan checks to CI.
