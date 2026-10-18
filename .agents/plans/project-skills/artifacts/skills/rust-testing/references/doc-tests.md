# Doc-tests

> **NOT YET USED IN THIS REPO.** Background for when the first doc-test is added. Source: *The rustdoc
> book, "Documentation tests"* (the `rust-docs` component of rustc **1.99.0**), read 2026-10-18.
> **[re-run]** = compiled and run in a scratch crate on rustc 1.99.0. Tag legend: `SKILL.md`.

## What a doc-test is

Every fenced code block in a doc comment (default language Rust) is compiled and run as a test; it
passes if it compiles and runs without panicking. Rustdoc preprocesses each block: inserts common
`allow` attributes, adds `extern crate <mycrate>;` (unless `#![doc(test(no_crate_inject))]`), and
wraps the text in `fn main() { ... }` unless it already contains `fn main`.

Doc-tests link only against the crate's **public** API. Test private items with unit tests.

## `?` in examples

A test body returns `()`, so `?` fails with a mismatched-types error. Three working forms:

```rust
/// ```
/// # fn main() -> std::io::Result<()> {   // hidden line: compiled, not rendered
/// let mut input = String::new();
/// std::io::stdin().read_line(&mut input)?;
/// # Ok(())
/// # }
/// ```
```

```rust
/// ```
/// let n: i32 = "42".parse()?;
/// # Ok::<(), std::num::ParseIntError>(())     // since 1.34.0; no `fn main` needed
/// ```
```

The second form must name the error type (the `?` conversion makes inference ambiguous), and the `(())`
must be written without whitespace so rustdoc recognises an implicit Result-returning function.
**[re-run]** The second form passed under `cargo test --doc`.

## Hidden lines and escaping

A line starting with `# ` is compiled but hidden in rendered docs. To show a literal leading `#`,
double it: `## bar` renders as `# bar`. Repeat the setup lines in each block so every block stays
independently testable while showing only the relevant part.

## Block attributes (comma/space separated, after the fence)

| Attribute | Effect | Note |
|---|---|---|
| `ignore` | not compiled, not run | the book: "almost never what you want"; add a reason in `(...)` |
| `no_run` | compiled, not run | for code that needs network or loops forever. **[re-run]** reported as "compile" |
| `should_panic` | must compile and panic | **[re-run]** |
| `compile_fail` | must fail to compile | see hazard below |
| `edition2015/2018/2021/2024` | compile with that edition | |
| `standalone_crate` | do not merge with other doctests | from the 2024 edition compatible doctests are merged for speed; each still runs in its own process |
| `ignore-<target>` | skip when the target name contains the text | e.g. `ignore-x86_64`; since 1.88.0 it overrides `ignore` |
| `test_harness` | run the `#[test]` functions inside instead of `main` | |

**Hazard (a vacuity trap, [re-run]):** `compile_fail` passes on *any* compilation error. A block whose
only problem is a typo in the snippet (`doctest_probe::add_twoo(1)`) reported `ok`. The book also warns
that code that fails today may compile in a future release. Treat a `compile_fail` block like any other
"X is rejected" assertion: pair it with a sibling block that compiles *without* the offending line, so
the failure can only be the one you meant. (See `must-fail-controls.md`.)

## Items that exist only while collecting doctests

`#[cfg(doctest)]` is set when rustdoc builds the crate for tests. Use it for hidden items that carry
`compile_fail` examples, or to test the README as part of the doctests:

```rust
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;
```

## Running them

- `cargo test --doc` runs only doc-tests; plain `cargo test` also includes them (both **[re-run]**).
- Show output / warnings: `cargo test --doc -- --show-output`; `#![warn(unused)]` in a block, or
  `#![doc(test(attr(warn(unused))))]` at the crate root, turns unused-code warnings back on.
- In a workspace, `--test-run-directory` (a rustdoc flag) separates the directory examples run in from the
  compile directory, so `fs` calls resolve relative to the crate.
- **cargo-nextest does not run doc-tests** (its own docs, quoted in `nextest-and-mutants.md`): keep a
  separate `cargo test --doc` step.

## Where they would fit here

Candidates are the pure crates (`thrust-contracts`, `thrust-ports`): small examples on types and trait
contracts. Snippets shared with the mdbook docs would use `{{#include}}` anchors; see the `mdbook`
skill if installed. They do not replace the contract suite: doc-tests cannot reach the HTTP layer or
any adapter.
