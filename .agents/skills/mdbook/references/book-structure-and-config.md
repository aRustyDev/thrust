# Book structure (SUMMARY.md), book.toml, and the 0.5 changes

Behaviour below is for mdBook v0.5.4 (`mdbook --version`) unless marked "documented" (the official guide page).
Tags: `[derived]` = from documentation, not executed; `[unverified: not run]` = not executed.

## SUMMARY.md grammar (documented: Format > SUMMARY.md)

```markdown
# Summary

[Introduction](intro.md)          <- prefix chapter: unnumbered, root level only, before numbered ones

# Guide                           <- part title: a level-1 heading only; other levels are ignored

- [Using add](guide/add.md)       <- numbered chapter; `-` or `*`, never mixed
  - [Nested page](guide/nested.md)  <- nesting by indentation
- [Draft chapter]()               <- draft: no path, rendered as a disabled link

---                               <- separator: a line of 3+ dashes only

[Contributors](contributors.md)   <- suffix chapter: unnumbered, root level only
```

Rules: the file must be named `SUMMARY.md` and sits in `src/` (or `book.src`); prefix and suffix
chapters cannot be nested; the parse is strict, unknown constructs are dropped or error.

## What the build does NOT tell you (0.5.4)

| Mistake | Behaviour | Guard |
|---|---|---|
| `.md` file under `src/` not linked from SUMMARY | silently omitted, **exit 0, no warning** | orphan check below |
| SUMMARY links a missing file, `create-missing = false` | `ERROR failed to read chapter`, **exit 101** | keep `create-missing = false` |
| Same, `create-missing` left at default `true` | mdBook creates a stub file in `src/` | set it `false` |
| Typo inside `[output.html]` (`addtional-css`) | `ERROR Failed to deserialize output.html ... unknown field`, **exit 0**, whole html table ignored (a configured `git-repository-url` disappeared from the output) | scan the log for `^ERROR` |
| Broken `{{#include}}` path | `ERROR Error updating "{{#include ...}}"`, **exit 0**, literal `{{#include ...}}` text ships in the page | scan the log for `^ERROR` |

Orphan check (run from the book root; prints files present on disk but not linked; empty = clean).
`mdbook init` writes links as `./chapter_1.md`, so the `sed` must strip an optional `./`; without it the check
reports every chapter of a clean book.

```sh
grep -oE '\]\([^)#]+\.md' src/SUMMARY.md | sed -E 's/^\]\((\.\/)?//' | sort -u > listed.txt
find src -name '*.md' ! -name SUMMARY.md | sed 's|^src/||' | sort -u > present.txt
comm -13 listed.txt present.txt
rm -f listed.txt present.txt
```

Cases exercised on mdBook 0.5.4: the `mdbook init` book (`./`-prefixed link) is clean; nested items with and
without `./` (`- [A](./guide/a.md)` then `  - [B](guide/b.md)`) are clean; a stray `orphan.md` is reported.
A page reached **only** through `{{#include partial.md}}` is also reported as an orphan, which fails safe: link
it from `SUMMARY.md` or move it out of `src/`. Links with spaces or URL-encoded characters are not handled.

Error-scan check (log lines start with `ERROR` at column 0; `INFO`/`WARN` are right-aligned):

```sh
mdbook build docs > build.log 2>&1
grep -E '^ERROR' build.log
```

A non-empty second command output is a failure even though `mdbook build` exited 0. In a CI `run:`
step use `if grep -qE '^ERROR' build.log; then ...; exit 1; fi` - `! grep` does not trip `set -e`.

## Minimal book.toml (builds on 0.5.4)

```toml
[book]
title = "Project docs"
authors = ["project contributors"]
language = "en"
src = "src"

[rust]
edition = "2021"        # match the linked crate; the default is 2015 and `async fn` blocks fail on it

[build]
build-dir = "book"
create-missing = false  # never auto-create stubs for dead SUMMARY links

[output.html]
git-repository-url = "https://github.com/<owner>/<repo>"
edit-url-template = "https://github.com/<owner>/<repo>/edit/main/docs/{path}"
site-url = "/<repo>/"   # Pages project sites live under /<repo>/; affects the 404 page
```

Documented keys: `[book]` title, authors, description, src, language, text-direction; `[rust]` edition
(2015/2018/2021/2024); `[build]` build-dir, create-missing, use-default-preprocessors, extra-watch-dirs;
`[output.html]` additional-css/js, search.*, fold, playground, smart-punctuation, definition-lists,
admonitions, and the git/edit URL keys. Prefer the generated list: a bad key makes mdBook print the valid
ones (the `unknown field ... expected one of` line).

## 0.5 changes that invalidate older advice (documented: CHANGELOG 0.5.0 - 0.5.4)

- Unknown fields in `book.toml` now error (but see the exit-0 trap above for `[output.html]`).
- Removed: `curly-quotes` (use `output.html.smart-punctuation`, now default `true`), `copy-fonts`,
  `book.multilingual`, google-analytics, legacy config.
- A preprocessor whose binary is missing is an error unless `optional = true` (exit 101 on 0.5.4).
- `mdbook test --dest-dir` removed; `build --dest-dir` is relative to the current directory.
- Added: `output.html.admonitions` (on by default, `> [!NOTE]` syntax), `definition-lists`,
  `sidebar-header-nav`, zoomable images; HTML ids are prefixed `mdbook-`; theme handlebars helpers
  changed (custom themes written for 0.4 need porting).
- Pin the version: the official install page recommends a SemVer range (`^0.5.4`) with `--locked`.
  This skill recommends an exact pin in CI (`=0.5.4`) so a docs build cannot break on a patch release.

## CLI surface (0.5.4, `--help`)

`mdbook init | build | test | clean | completions | watch | serve`. `test` takes `-c/--chapter` and
`-L/--library-path` (comma-separated or repeated). `build` takes `-d/--dest-dir` and `-o/--open`.
