# Anti-pattern gate (stage 6)

Run after verify, per slice, and record the result in the ledger row. Two parts: lints a machine can enforce,
and a short review list for what lints cannot see. The gate can fail: a crate with one planted violation of each
lint fails it and a clean crate passes it `[verified: clippy 0.1.98]`.

## 1. Machine-enforced

```
cargo clippy --lib -- -D warnings \
  -D clippy::unwrap_used -D clippy::expect_used -D clippy::indexing_slicing -D clippy::string_slice \
  -D clippy::cast_possible_truncation -D clippy::cast_sign_loss -D clippy::cast_precision_loss \
  -D clippy::float_cmp -D clippy::todo -D clippy::dbg_macro
```

The planted-violation crate covers indexing, string slicing, `unwrap`, `expect`, float-to-int truncation, a
sign-losing cast, a precision-losing cast, float `==`, `todo!`, `dbg!`, `&Vec`/`&String` parameters and `clone` on
a `Copy` type; it fails with 13 errors, and the same flags pass the clean crate `[verified: clippy 0.1.98]`.

| Lint | What it stops | Port-specific reason |
|---|---|---|
| `unwrap_used`, `expect_used` | panics on user input | JS threw catchable errors; Rust must return them |
| `indexing_slicing`, `string_slice` | out-of-range and mid-character panics | `&s[0..1]` panics on `"😀"` (JS returns a lone surrogate) |
| `cast_*` | silent wrap, saturate, sign loss | JS `\|0` wraps; Rust float `as` saturates |
| `float_cmp` | `==` on floats | JS `===` habits |
| `todo` | leftover placeholders | allowed only in slices marked defer |
| default `await_holding_lock` | a `MutexGuard` held across `.await` | on by default in clippy `[verified: clippy 0.1.98]` |

Notes:

- **Run on library targets (`--lib`, `--bins`), not `--all-targets`.** Test code indexes and unwraps by design,
  and these lints fire there `[verified: clippy 0.1.98]`.
- A range-checked float-to-int cast still trips `cast_possible_truncation`. Allow it locally with a reason:
  `#[allow(clippy::cast_possible_truncation, reason = "range checked above")]`; a `reason` makes the exception
  greppable.
- Shared-ownership smells (`Arc<Mutex<_>>`, `Rc<RefCell<_>>`) are caught by the review grep below, not by these
  lints.

## 2. Review checklist (grep, then read)

```
grep -rnE 'Arc<(Mutex|RwLock)|Rc<RefCell|Box<dyn (std::error::)?Error|serde_json::Value' src
```

The grep flags all five patterns when planted and prints nothing on a clean crate
`[verified: grep (BSD grep, GNU compatible) 2.6.0-FreeBSD]`. It matches `serde_json::Value` only when written out in
full, so also look for an imported `Value` in signatures. Every hit needs a one-line justification in the review,
or a fix:

| Smell | Why it appears in ports | Fix |
|---|---|---|
| `Arc<Mutex<_>>`, `Rc<RefCell<_>>` object graphs | mirroring shared mutable JS objects | decide ownership; message passing or a smaller shared core |
| `.clone()` scattered to satisfy the borrow checker | translating aliasing line by line | borrow, restructure, or clone once at a real boundary |
| `serde_json::Value` (or an imported `Value`) in a public signature | `any` / `unknown` carried through | typed structs; `Value` only at the entry point |
| `Box<dyn Error>` in a library | `catch (e)` with no type | a `thiserror` enum |
| `String` where a closed set exists | string-literal unions left as strings | `enum` |
| `HashMap` in serialised output | `{}` objects | `BTreeMap`/`IndexMap`; order is observable (`json-fidelity.md`) |
| `#[serde(untagged)]` without an ordering test | first match silently wins | tag it, or add a case per variant |
| `#[serde(other)]` on a type that is re-serialised | lossy round trip | keep the original string |
| class hierarchy mirrored as structs, getters/setters everywhere | transliteration | traits and plain data |
| one `utils` module of mirrored helpers | file-shape porting | move each helper next to its only caller |
| `tokio::spawn` or a global runtime inside a library | `Promise` habit | take collaborators as trait parameters |
| `unsafe` | rarely justified in a TS port | remove, or document the invariant above the block |

## 3. Placeholders

A deferred slice's stub must compile, fail **loudly**, and have a ledger row naming the missing behaviour, the
blocker and the owner. If the stub is reachable from user input, return an explicit `Err(Unsupported)` instead of
`todo!()` (a panic is a denial-of-service). A stub that quietly returns a default is the failure to avoid. The gate
counts `todo!` with `grep -rn 'todo!' src`: the number of lines must equal the number of deferred stubs written
with it (a stub that returns an explicit error is recorded in the ledger instead). Because `-D clippy::todo`
rejects every `todo!`, confine such stubs to their own module under
`#![allow(clippy::todo, reason = "deferred, see ledger")]`. The command prints nothing for the worked-example port
and two lines for a module with two `todo!` stubs `[verified: grep (BSD grep, GNU compatible) 2.6.0-FreeBSD]`.

## 4. Result format

`gate: clippy --lib clean | N review hits, all justified | todo! count = deferred todo! stubs`. Anything else is a
failed gate; fix it or open a ledger row, never waive it silently.
