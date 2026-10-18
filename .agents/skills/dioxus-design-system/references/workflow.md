# `dx components` workflow (Dioxus CLI 0.7.10)

Verified against dx 0.7.10 (57d6794): `dx components --help`, the CLI source at tag `v0.7.10`
(`packages/cli/src/cli/component.rs`, `packages/cli/src/config/component.rs`), the registry's
`component.json` manifests, and the output of `dx components schema` and `dx config schema`. Registry
facts are for `DioxusLabs/dioxus-components` `main` (commit `9a75825`, 2026-09-08) as of 2026-10-18. Not
exercised against the live registry: online `list` and `add` (they clone a git repository), so their file
effects come from the source.

## Repository identity

- Official repository: `https://github.com/DioxusLabs/dioxus-components` (default branch `main`;
  GitHub metadata reports Apache-2.0 and the README says dual MIT or Apache-2.0).
- The CLI's built-in default registry URL is `https://github.com/dioxuslabs/components`, which GitHub
  redirects (301) to `dioxus-components`; the former name is the same repository. Registry `component.json`
  files still declare their dependency on the old URL.
- Layout: `primitives/` (the unstyled `dioxus-primitives` crate), `preview/` (styled components and the
  gallery app; `preview/src/components/<name>/` is what `add` copies), `playwright/`, `test-harness/`,
  `dioxus-attributes/`, `component.json` (registry index).

## Commands

| Command | What it does (from source) |
|---|---|
| `dx components list [--git U] [--rev R] [--path P]` (user-run) | clones the registry into a cache if absent, prints `- <name>: <description>` per component |
| `dx components add <names...> [--all] [--module-path D] [--global-assets-path D] [--force] [--git U] [--rev R] [--path P]` | see "What add does" |
| `dx components update [--git U] [--rev R]` | fetches the cached registry and hard-resets the **cache** checkout to the latest default-branch commit, or to the rev when `--git` is also given; without `--git` the rev is dropped (see "`--rev` needs `--git`"). With no flags it uses the registry configured in `Dioxus.toml` (`config.components.registry.remote`, line 218), so a configured `git` + `rev` is reset to that rev. Does not touch the project |
| `dx components remove <names...> [--all]` | joins each name to the components directory and calls `remove_dir_all` on the result with no validation, then removes the `pub mod <name>;` line in `mod.rs`; it does not remove Cargo dependencies or the theme. Pass only real component names |
| `dx components clean` | deletes the registry cache directory |
| `dx components schema` | prints the JSON schema for component manifests (offline) |

Global flags apply: `--offline` (fails with `Cannot download component registry '<url>' while offline`
when the cache is empty), `--locked`, `--frozen`, `--json-output`, `--verbose`. Defaults: components go to
`src/components` (`--module-path`), global assets to `[application] asset_dir` or `assets`
(`--global-assets-path`). The command must run inside a Cargo project (it calls `cargo metadata`).

`list`, `add`, `remove` and `update` clone, change or delete files, and `add` runs `cargo`: the agent
proposes each for the user's approval and does not run them.

## Registry selection and `Dioxus.toml`

```toml
[components]
components_dir = "src/ui"            # default: src/components

[components.registry]                # default: the built-in registry, latest default branch
git = "https://github.com/DioxusLabs/dioxus-components"
rev = "<40-hex SHA>"                 # or: path = "../local-registry"
```

Shape verified from `dx config schema`: `components.components_dir` (string) and
`components.registry.{git, rev, path}` (optional strings). CLI flags override it. **A `git` or `path`
here silently changes which source `dx components add` copies.** Read it before any add; anything other
than the official repository (or its former name) is a third-party registry (below).

## `--rev` needs `--git`

In the 0.7.10 source, `RemoteComponentRegistry::resolve_or_default` returns the pair `(git, rev)` only when
`--git` is given; otherwise it returns the built-in URL with **no rev** (`component.rs`, lines 424-430).
So `dx components add button --rev <sha>` should silently ignore the rev and copy the default registry's
latest `main`. Because `is_default()` (line 491) is also false whenever a rev is set, the command
additionally skips the `[components.registry]` setting in `Dioxus.toml`.
`[unverified: source reading, not run. A probe the user can run in a throwaway crate: dx components list
--rev deadbeef would fail with "Failed to find revision" if the rev were honoured.]`

Therefore every pinned command in this skill is
`dx components add <names> --git https://github.com/DioxusLabs/dioxus-components --rev <40-hex>`.

## What `add` does, in order

1. Resolve config and registry; read every `component.json`.
2. Look up the requested names (`--all` takes every component). Unknown name: `Component '<name>' not
   found in registry`.
3. Ensure the components module exists (creates `src/components/mod.rs` when new).
4. Resolve **component dependencies** recursively (built in, or third party given as `git` + `rev`).
5. Collect **cargo dependencies** from every involved manifest and run `cargo add` for each as a
   subprocess (`dioxus-component-manifest`, `CargoDependency::add_command`): a plain-string entry runs
   `cargo add <name>` with no version, and an object entry adds `--features`, `--git` and `--rev` as given
   plus `--no-default-features` unless the entry sets `default_features: true`. For the default registry this
   is `dioxus-primitives` from `https://github.com/DioxusLabs/components` with **no rev**, so the project's
   `Cargo.toml` tracks a moving branch until it is pinned.
6. For each component, in an unspecified order (the set is a `HashMap`, lines 143 and 188): copy its
   files into `<components_dir>/<name>/` (the manifest's `exclude` list is not copied), then copy its
   `globalAssets` (for example `dx-components-theme.css`) into the assets directory, then append
   `pub mod <name>;` to `mod.rs` (`add_component`, lines 575-625). An existing destination fails with
   `Destination directory '<path>' already exists` for a requested component unless `--force` is given;
   for a component pulled in as a dependency an existing directory is skipped silently (lines 592-598 and
   651-657), assets included.
7. Print finishing steps, but only conditionally: the "add `mod components;` to `main.rs`" line appears
   only when the components module was **newly created**, and the "reference
   `asset!("/assets/dx-components-theme.css")` as a stylesheet" line additionally requires the registry in
   effect to be the built-in default (`registry.is_default()`, line 208), so **any `--git`, `--rev` or
   `--path`, or a `[components.registry]` setting, suppresses it**. On later adds nothing is printed.
   Always check both yourself. (The registry README says `/assets/dx-components.css`; the CLI and
   manifests use `dx-components-theme.css`.)

The registry cache is keyed by URL and rev and, once cloned, reused by every later `add` and `list`
until `dx components update` runs: "the registry at the time of the add" means the cache as last cloned
or updated, not necessarily the current remote HEAD.

Once copied, the source is the project's own: edit freely. A re-add without `--force` fails with
`Destination directory ... already exists` and leaves the edits alone; only `add --force` overwrites them
and `remove` deletes the directory. Keep local changes in your own stylesheet where possible (`theme.md`).

## Choosing and recording the SHA

`--rev` is checked out with git `revparse`, so branch names, tags and short SHAs all work technically.
Require a full 40-hex SHA anyway: branches and tags move and short SHAs can become ambiguous.

1. Ask the user for the SHA of `DioxusLabs/dioxus-components` they have reviewed. If they have none, they
   can choose the current `main` commit (the repository page shows it, or the user can run
   `git ls-remote https://github.com/DioxusLabs/dioxus-components refs/heads/main`) and then review it.
2. At that SHA, read `component.json` for each requested component. `cargoDependencies` should name only
   expected crates (`dioxus-primitives`, and sometimes `dioxus-icons`, `time`, `palette` and similar);
   `componentDependencies` lists other registry components that will be copied too.
3. At that SHA, read the workspace `Cargo.toml` `dioxus` requirement (`0.7.8` on `main` at the read date)
   and make sure the project's Dioxus is not older.
4. Record the SHA: in the commit message and as the `rev` in `Cargo.toml`.

## Pinning and verifying (supply chain)

Two moving pieces are unpinned by default: the copied source (the registry cache's `main`) and the git
dependency on `dioxus-primitives`. To make a build reproducible:

1. `dx components add <names> --git https://github.com/DioxusLabs/dioxus-components --rev <40-hex>`
   (user-run, after approval).
2. **Mandatory after every add, and after any aborted add: compare with the SHA.**
   - Method: the user clones `https://github.com/DioxusLabs/dioxus-components` somewhere outside the
     project and checks out the recorded SHA; the agent compares with Read and Grep (or the user runs a
     recursive diff).
   - Scope: (a) each `<components_dir>/<name>/` against `preview/src/components/<name>/` in the checkout;
     (b) the assets directory's `dx-components-theme.css` against `preview/assets/dx-components-theme.css`;
     (c) the `Cargo.toml` changes and `Cargo.lock`. `cargo add` also runs for the cargo dependencies named by
     the manifests of built-in dependencies, which come from the default registry and are not pinned, even
     when the dependency-first approach below is used.
   - Expected differences: only the entries in each component's manifest `exclude` list are not copied (for
     Button: `variants`, `docs.md` and `component.json`), plus the project's own later edits. Anything else
     means something came from somewhere other than the reviewed SHA.
3. Fix the dependency `dx` added, keeping whatever keys it wrote (for example `default-features = false`):
   change only the URL to `https://github.com/DioxusLabs/dioxus-components` and add `rev = "<40-hex>"`.
4. Commit `Cargo.lock`; review the generated diff like any vendored dependency.

**Known gap (DioxusLabs/dioxus#5310, open; draft fix PR #5309 open, when read):** `--git` and `--rev` do not
apply to built-in *component* dependencies: they resolve from the default registry's cache. In the 0.7.10
source, `registry_root` is the pinned registry's cache (line 114) and is passed to every component's
`add_component` (line 190); `copy_global_assets` (lines 789-813) refuses any global asset outside that root.
Every dependency of `color_picker`, `date_picker`, `item` and `sidebar` declares
`globalAssets: ["../../../assets/dx-components-theme.css"]`, so for a dependency that is **not already
present** the add should **abort** with

`Cannot copy global asset '...' for component '...' because it is outside of the component registry '...'`

(line 808), not silently mix versions. The abort comes after `cargo add` has already changed `Cargo.toml`
(line 185), and after that dependency's files were copied but before its `pub mod` line was appended (lines
602-622); because components are processed in an unspecified order, other components in the same command
may or may not have been copied. The requested component itself, if present, is read from the pinned SHA.
Affected components on `main` (they declare `componentDependencies`): `color_picker` (popover, input),
`date_picker` (calendar, popover), `item` (separator), `sidebar` (button, separator, sheet, skeleton,
tooltip); none of those dependencies has dependencies of its own.
`[unverified: source reading and issue report, not run]`

**Primary approach for those four: add the dependency components first**, each as a separate earlier
command with the same `--git` and `--rev` (for `color_picker`: `popover` and `input`, then `color_picker`).
The dependencies then copy from the pinned registry, and when the dependent is added they already exist and
are skipped, so the asset check does not run for them. Do not request a dependency and its dependent in one
command: the dependent's default-registry copy of the dependency may be processed first
`[inferred from the unordered map]`. Then run the mandatory comparison. If an abort has already happened,
use the triage row below.

## crates.io name collisions (as of 2026-10-18)

- `dioxus-primitives` on crates.io is version `0.0.0`, created 2025-04-13, with no repository link: a name
  reservation, not the library. The in-repo crate is `0.0.1` and unpublished. The registry README badge
  links to the crates.io page; do not read that as "published".
- `dioxus-components` on crates.io (0.1.2) comes from `LeTri234/dioxus_component`, an unrelated
  third-party project with a similar name.
- Other crates named by registry manifests are ordinary dependencies and are legitimate to take from
  crates.io.

## Third-party registries (`--git <other url>` or `--path`)

A registry is source code that gets copied into the project plus `cargo add` commands the registry chooses.
Treat it as untrusted: pin `--rev`, read every manifest and the diff, and get explicit approval for the
exact command.

## Manifest (`component.json`) keys

From `dx components schema`: `name`, `description`, `authors`, `exclude` (string list),
`cargoDependencies` (each a crate-name string or an object `{ name, version?, git?, rev?, features?,
default_features? }`), `componentDependencies`, `globalAssets` (string list), `members` (registry index
entries). Example (Button): depends on `dioxus-primitives` via `git` with no `rev` and lists
`../../../assets/dx-components-theme.css` as a global asset.

## Version baseline

The registry workspace declares `dioxus = "0.7.8"` and the styled components use the `#[css_module("...")]`
attribute macro. It has not been checked against a 0.8 CLI or 0.8 alpha projects; use a 0.7 CLI for
component work and compile one component before relying on a newer toolchain.

## Failure triage

| Symptom | Likely cause |
|---|---|
| `dx` prints a `deno` banner | not the Dioxus CLI; fix `PATH` first |
| `could not find Cargo.toml` | run inside the crate or workspace |
| `Cannot download component registry ... while offline` | empty cache plus `--offline`/`--frozen`; run once online or use `--path` |
| `Destination directory ... already exists` | a requested component is already present (an earlier add, or a partial copy from an aborted one); a re-add without `--force` leaves it alone, only `--force` overwrites it (local edits lost) and `remove` deletes it |
| `Cannot copy global asset '...' for component '...' because it is outside of the component registry '...'` | #5310: a built-in dependency was resolved from the default registry's cache while the pin is another registry; see "Cleanup after an aborted add" |
| `Component '<name>' not found in registry` | wrong name; the user can run `dx components list` (see `mapping.md`) |
| the copied files differ from the SHA | `--rev` given without `--git`, or a built-in dependency taken from the default registry (#5310) |
| compile errors in a copied component | project Dioxus older than the registry baseline, or registry HEAD newer than the pinned `dioxus-primitives` rev; pin both to one SHA |
| unstyled components | theme stylesheet not linked, or `mod components;` missing |

## Cleanup after an aborted add

The state after the #5310 abort (source reading, not run): `cargo add` has already changed `Cargo.toml`
(and possibly `Cargo.lock`); one or more `<components_dir>/<name>/` directories may exist; none of the
copied components that had not finished is listed in `mod.rs`; the theme file may or may not have been
copied. Propose these steps and let the user run them:

1. Review `git status` and `git diff` to see exactly what changed in `Cargo.toml`, `Cargo.lock`, the
   components directory and the assets directory. Revert unwanted `Cargo.toml` changes.
2. Delete the partially copied component directories, or have the user run `dx components remove <name>`
   for each (it runs `remove_dir_all` on the name it is given, so only real component names).
3. Re-run with the dependency components first (see "Known gap"). A re-run of the dependent fails with
   `Destination directory ... already exists` if its directory survived step 2; existing dependency
   directories are skipped; only `--force` overwrites.
4. Run the mandatory comparison with the SHA before committing.
