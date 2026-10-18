# Sources: mdbook

External sources are cited for facts and short syntax snippets only; no third-party skill text is reproduced.
All retrieved 2026-10-18.

| Source | Supports | Licence |
|---|---|---|
| https://rust-lang.github.io/mdBook/format/summary.html | `SUMMARY.md` grammar | MPL-2.0 |
| https://rust-lang.github.io/mdBook/format/configuration/general.html | `book.toml` keys | MPL-2.0 |
| https://rust-lang.github.io/mdBook/format/configuration/preprocessors.html | `command`, `before`, `after`, `renderers`, `optional` | MPL-2.0 |
| https://rust-lang.github.io/mdBook/cli/test.html | `mdbook test -L`, chapter selection, ignore/untagged rules | MPL-2.0 |
| https://rust-lang.github.io/mdBook/format/mdbook.html | `{{#include}}` anchors and ranges, `rustdoc_include`, code-block attributes, hidden lines | MPL-2.0 |
| https://rust-lang.github.io/mdBook/continuous-integration.html (shows v0.5.4) | SemVer pin advice, `--locked` | MPL-2.0 |
| https://github.com/rust-lang/mdBook/blob/master/CHANGELOG.md (0.5.0 - 0.5.4) | 0.5 breaking changes | MPL-2.0 |
| https://github.com/rust-lang/mdBook/wiki/Automated-Deployment%3A-GitHub-Actions | Pages permission set (`pages: write`, `id-token: write`) | wiki text; the install method is not used |
| https://github.com/badboy/mdbook-mermaid and https://crates.io/api/v1/crates/mdbook-mermaid (0.17.1) | what `install` writes; `mdbook-preprocessor` ^0.5.0 | MPL-2.0 |
| https://github.com/tommilligan/mdbook-admonish and https://crates.io/api/v1/crates/mdbook-admonish (1.20.0) | `install`, asset-version failure; `mdbook` ^0.4.40 | MIT |
| https://github.com/Michael-F-Bryan/mdbook-linkcheck and https://crates.io/api/v1/crates/mdbook-linkcheck (0.7.7) | maintenance state; `mdbook` ^0.4 | MIT |
| https://github.com/marxin/mdbook-linkcheck2 and https://crates.io/api/v1/crates/mdbook-linkcheck2 (0.13.0) | `[output.linkcheck2]` keys, `book/html/` output; `mdbook-driver`/`mdbook-renderer` ^0.5.1 | MIT |

## Verification status

| Claim group | Status |
|---|---|
| Exit status of a broken `{{#include}}`, a mistyped `[output.html]` key, an orphan chapter; `create-missing`; missing preprocessor with and without `optional`; native `> [!NOTE]` | executed on mdBook 0.5.4 |
| E0432 and E0464 with their fixes; `#[cfg(doctest)]` chapter doc-tests; `examples/` anchor include; `mdbook test` exit 101 | executed with rustc/cargo 1.98.1 on a minimal crate and book |
| Orphan check: `mdbook init` book with `./` links, nested links with and without `./`, an injected orphan, an include-only page | executed on mdBook 0.5.4 |
| `examples/docs.yml` | linted with actionlint 1.7.12 plus a must-fail control; `[unverified: not run]` on GitHub |
| mermaid, admonish, linkcheck, linkcheck2 behaviour and mdBook 0.5 compatibility | `[unverified: not run]`; library requirements are from crate manifests |
| Anything in a multi-crate workspace | `[unverified: not run]` |
