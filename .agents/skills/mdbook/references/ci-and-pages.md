# CI and GitHub Pages for an mdBook

Worked file: `examples/docs.yml` in this directory. It is a template: set `BOOK_CRATE` to the crate the chapters
import (the placeholder value `your-crate` fails the build on purpose). Adjust the `paths:` filter to every
directory the book includes (for example `examples/**`), because a pull request that touches only an included
example would otherwise skip the check; `cargo build --locked` needs a committed `Cargo.lock`. A manual
`workflow_dispatch` run builds and tests but never publishes. Lint status and which steps were not executed are
listed once in `SOURCES.md`.

## What the workflow does, and why

| Choice | Reason |
|---|---|
| `permissions: {}` at top, `contents: read` on `build`, `pages: write` + `id-token: write` only on `deploy` | least privilege; the deploy action needs exactly those two scopes |
| `cargo install mdbook --locked --version "=0.5.4"` on every run, no cached executable | the mdBook version is exact (the Rust toolchain floats unless pinned); a restored binary would be executed by a job that can publish; the upstream wiki recipe downloads `releases/latest` and so changes under you |
| actions pinned to full commit SHAs with a `# vX.Y.Z` comment | tags are mutable; the SHAs are a 2026-10-18 snapshot, re-resolve when bumping |
| `CARGO_TARGET_DIR=target/book` + `-L target/book/debug/deps` | one rlib per crate, so no E0464 |
| log scan for `^ERROR` | `mdbook build` exits 0 on a bad include and on a mistyped `[output.html]` key |
| orphan check (strips an optional `./`) | chapters missing from `SUMMARY.md` vanish silently |
| `mdbook test` as its own step | separates "book builds" from "examples compile" in the check list |
| `cancel-in-progress` only for `pull_request`; `pages` group never cancels | cancelling a half-finished Pages deploy leaves the site stale |
| upload + deploy gated on `push` to `main` | PRs build and test but never publish |

Repository setting required once: **Settings > Pages > Source = "GitHub Actions"**. Without it, `deploy-pages`
fails even though the workflow is valid.

## Paths and the deploy artifact

- Artifact `path:` is the book's `build-dir` (`docs/book`). If a second `[output.*]` backend (linkcheck) is
  enabled, HTML moves to `docs/book/html`; change the path.
- A project site is served under `/<repo>/`; set `[output.html] site-url = "/<repo>/"` so the 404 page links
  resolve. A custom domain at the root uses `site-url = "/"`.

## Required checks

The `paths:` filter means a PR touching none of those paths produces no `docs` check. That suits an optional
check. A path-filtered workflow made a *required* check blocks every PR that skips it; design required checks
with the repository's CI workflow-design guidance, if present, rather than here.

## Version bumps

- mdBook: edit `MDBOOK_VERSION`, read the release notes for the minor, run the guards locally (`mdbook build`,
  log scan, orphan check, `mdbook test`), then commit.
- Actions: re-resolve the tag to a SHA (`git ls-remote https://github.com/<owner>/<repo> 'refs/tags/<tag>*'`;
  use the peeled `^{}` commit for annotated tags). Dependabot with `package-ecosystem: "github-actions"`
  automates it.

## Not reproduced

The upstream "Automated Deployment: GitHub Actions" wiki recipe installs mdBook with
`curl .../releases/latest | tar -xz` and floats action majors (`@v4`): an unpinned download of executable
content. This workflow does neither.
