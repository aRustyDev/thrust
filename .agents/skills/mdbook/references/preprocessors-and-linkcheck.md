# Preprocessors, admonitions, diagrams, link checking

Behaviour is for mdBook 0.5.4. Crate facts are from the crates.io API as of 2026-10-18. Tags: `[derived]` =
from documentation or manifests, not executed; `[unverified: not run]` = not executed.

## How preprocessors are wired

```toml
[preprocessor.mermaid]
command = "mdbook-mermaid"      # default is mdbook-<name>
# renderers = ["html"]          # restrict to a renderer
# before = ["links"]            # ordering; `after = [...]` is the mirror
# optional = true               # missing binary becomes a WARN instead of an error
```

With `[preprocessor.mermaid]` and no binary on PATH (0.5.4):

- without `optional`: `ERROR The command 'mdbook-mermaid' wasn't found ...`, **exit 101**.
- with `optional = true`: `WARN ... was not found, but is marked as optional`, build succeeds.

**Do not set `optional = true` in CI for a preprocessor that rewrites content** (mermaid, admonish): the build
goes green and ships raw fenced blocks. It is acceptable only for contributor convenience in a local-only
profile. Built-in preprocessors are `links` (the `{{#include}}` family) and `index`;
`[build] use-default-preprocessors = false` disables them (do not).

Installing any of the tools below, or running `cargo install`, is the user's decision.

## Admonitions: usually no preprocessor on 0.5

mdBook 0.5 renders GitHub-style callouts natively (`output.html.admonitions`, default on). On 0.5.4,
`> [!NOTE]` and `> [!WARNING]` produced `class="blockquote-tag blockquote-tag-note"` / `...-warning` with a
title element, with no preprocessor configured.

```markdown
> [!NOTE]
> Native admonition; renders the same on GitHub.
```

`mdbook-admonish` adds fenced syntax (` ```admonish warning "Title" `), custom types and collapsible blocks.
Adopt it only if those are needed. Release 1.20.0 (2025-06-06, MIT) depends on the `mdbook` **0.4.40**
library (crate manifest), i.e. it predates 0.5; compatibility with 0.5.x is untested `[unverified: not run]`.
`mdbook-admonish install <book dir>` writes `mdbook-admonish.css` and edits `book.toml`; re-run `install` after
every minor upgrade, because a stale asset version fails the build with `Incompatible assets installed ...
Please run mdbook-admonish install` (documented).

## Diagrams: mdbook-mermaid

- Release 0.17.1 (2026-08-14, MPL-2.0) depends on `mdbook-preprocessor` ^0.5.0 (crate manifest), so it targets
  0.5. Mermaid itself is MIT (README).
- `mdbook-mermaid install <book dir>` copies **`mermaid.min.js` and `mermaid-init.js` into the book directory**
  and adds to `book.toml`:

```toml
[preprocessor.mermaid]
command = "mdbook-mermaid"

[output.html]
additional-js = ["mermaid.min.js", "mermaid-init.js"]
```

- Consequence: a minified third-party script is committed. Review its diff on every upgrade, pin the tool
  version in CI and keep the file out of lint/format globs `[unverified: not run]`.
- Usage is a plain fence: ` ```mermaid `. `mdbook test` skips it (non-Rust language tag).
- Alternative with no tool and no committed JS: export the diagram to SVG and embed `![alt](diagram.svg)`;
  slower to edit, no supply-chain surface.

## Link checking

mdBook has **no built-in link checker**.

| Crate | Release (crates.io) | Depends on | Licence |
|---|---|---|---|
| `mdbook-linkcheck` | 0.7.7, 2022-10-03 (last release) | `mdbook` ^0.4 | MIT |
| `mdbook-linkcheck2` (maintained fork) | 0.13.0, 2026-08-16 | `mdbook-driver` / `mdbook-renderer` ^0.5.1 | MIT |

So `mdbook-linkcheck` targets 0.4 and `mdbook-linkcheck2` targets 0.5; neither was exercised against a real book
`[unverified: not run]`. The fork's table is `[output.linkcheck2]` (`output.linkcheck` is still read).

```toml
[output.html]

[output.linkcheck2]
follow-web-links = false             # default; web checks are slow and flaky in PR CI
traverse-parent-directories = false  # a link may not escape the book root
warning-policy = "error"             # default "warn" lets broken links through
exclude = ['localhost']              # regexes applied to the href
```

Documented in the fork's README:

- Enabling a second `[output.*]` makes mdBook write each renderer into its own sub-directory: HTML moves from
  `book/` to **`book/html/`**. Update the Pages artifact path or the deploy uploads the wrong tree.
- The checker runs as a renderer during `mdbook build`, so a broken link can fail that command itself `[derived]`.
- `http-headers` values interpolate `$ENV` variables; never put tokens there.
- Run external-link checks (`follow-web-links = true`) from a **scheduled** job, not on every PR.

Without a checker, two checks are already free: the `^ERROR` log scan catches a broken `{{#include}}`, and the
orphan check catches unlinked chapters (`book-structure-and-config.md`). Broken in-page links between chapters
are covered by neither.
