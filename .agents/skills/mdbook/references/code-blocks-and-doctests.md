# Code blocks, `mdbook test`, and sharing snippets with doc-tests

Behaviour is for mdBook 0.5.4 with rustc/cargo 1.98.1 and `edition = "2021"` under `[rust]`; the examples use a
crate named `demo`. Tags: `[derived]` = from documentation, not executed; `[unverified: not run]` = not executed.

## Code-block attributes (documented: Format > mdBook-specific features)

`rust` (default for untagged blocks - **untagged blocks are tested too**), `ignore`, `no_run`,
`should_panic`, `compile_fail`, `editable`, `noplayground`, `mdbook-runnable`, `edition2015|2018|2021|2024`.
Hidden lines: prefix with `# ` (hash-space); write `##` for a literal `#`. Blocks tagged with another
language (`toml`, `sh`, `text`, `console`) are skipped by `mdbook test`.

## `mdbook test` against a crate

```sh
cargo build                                   # produces target/debug/deps/libdemo-<hash>.rlib
mdbook test docs -L target/debug/deps         # exit 0 => every rust block compiled/ran
```

| Symptom | Cause | Fix |
|---|---|---|
| `error[E0432]: unresolved import 'demo'` ... `use of unresolved module or unlinked crate` | edition 2018+ extern prelude does not include `-L` crates | add a hidden first line `# extern crate demo;` to the block |
| `error[E0464]: multiple candidates for 'rlib' dependency 'demo' found` | two builds of the crate (e.g. different `--features`) left two `libdemo-<hash>.rlib` in `deps/` | `cargo clean -p demo`, then build **one** configuration; or give book tests a dedicated `CARGO_TARGET_DIR` |
| `mdbook test` exit **101**, block listed `FAILED` | block does not compile or panics | read the rustdoc output above the exit |
| Third-party dependency of the crate not found | documented recommendation is `-L target/debug/deps/`, which holds dependency rlibs too | build with the same profile you test with (`--release` goes to `target/release/deps`) `[unverified: not run with third-party dependencies]` |

`-L` accepts a comma list or repeats (`-L a -L b`). `--dest-dir` no longer exists on `test` in 0.5.
`-c <chapter>` tests one chapter by name or path. `mdbook test` does not need network.

## Sharing code between the book and the crate

Three mechanisms, in order of preference.

### 1. Include an anchored region of a cargo-compiled file

Anchor comments in the Rust file (name matches `[\w_-]+`):

```rust
// examples/quickstart.rs  (compiled by `cargo test` / `cargo build --examples`)
use demo::add;
fn main() {
    // ANCHOR: quickstart
    let total = add(20, 22);
    assert_eq!(total, 42);
    // ANCHOR_END: quickstart
}
```

In the chapter, path is **relative to the chapter file**, not the book root:

````markdown
```rust,ignore
{{#include ../../../examples/quickstart.rs:quickstart}}
```
````

`ignore` is deliberate: cargo already compiles the file, so `mdbook test` need not (and would need
`-L`). The snippet cannot drift from compiled code. Line-range forms also work:
`file.rs:2`, `file.rs::10`, `file.rs:2:`, `file.rs:2:10`. Prefer anchors; line numbers rot.

### 2. Make the chapter itself a doc-test (no `-L`, no `extern crate`)

```rust
// src/lib.rs
#[cfg(doctest)]
#[doc = include_str!("../docs/src/guide/add.md")]
pub struct GuideAdd;
```

`cargo test --doc` then compiles every Rust block in that chapter with the crate already in scope
(on a minimal chapter: passed, one `ignore` block skipped, including a `rust,editable` block and a
`compile_fail` block). Use this when you want the crate's own `cargo test` to gate the docs. Limits:
`{{#include}}` directives are not expanded by rustdoc, so blocks that use them must be `rust,ignore`;
the item only exists under `cfg(doctest)`, so it is invisible to users.

### 3. `{{#rustdoc_include}}` to show part of a block, run all of it

````markdown
```rust
# extern crate demo;
{{#rustdoc_include ../../../examples/quickstart.rs:2:9}}
```
````

Lines inside the range show normally; the rest of the file is included but hidden with `# `.

## Do not

- Paste long code into chapters "for now"; use mechanism 1.
- Leave a block untagged when it is not Rust (it gets compiled and fails).
- Rely on `mdbook build` to catch a bad `{{#include}}`; it exits 0 (see
  `book-structure-and-config.md`).
- Expect mdBook to know about your `Cargo.toml`: it does not. `-L` is the only link between the
  book and a crate.

## In a multi-crate workspace

Link only the crate(s) the chapters import, built in a **dedicated target dir** so feature or test
builds in the normal `target/` cannot create a second rlib:

```sh
CARGO_TARGET_DIR=target/book cargo build -p demo
mdbook test docs -L target/book/debug/deps
```

`[unverified: not run against a multi-crate workspace]`
