# TypeScript to Rust: type and idiom map

Stage 3 (Redesign). The Rust snippets at the end compile and pass `cargo clippy -D warnings` plus the gate lints
in `anti-patterns.md` `[verified: rustc 1.98.1, clippy 0.1.98, serde 1.0.229, serde_json 1.0.154, thiserror 2.0.21]`.
Wire-shape questions (null vs missing, tagged vs untagged, numbers, dates, Unicode) live in `json-fidelity.md`;
this page is about design.

Rule of thumb: port the **observable behaviour and the data model**, not the file shape. Where TypeScript's type
system was unsound or stringly typed, the port tightens it, and each tightening that changes behaviour is a row
in the exception register (`freeze-and-oracle.md`), never a silent change.

## 1. Types

| TypeScript | Rust | Notes |
|---|---|---|
| `"a" \| "b"` string-literal union | `enum` + `#[serde(rename_all = ...)]` | closed set; exhaustive `match` replaces `switch` plus a `never` check |
| `{kind: "circle", r} \| {kind: "square", side}` | `enum` with data, `#[serde(tag = "kind", rename_all = "lowercase")]` | see snippet A |
| `interface` with only data | `struct` | derive `Debug, Clone, PartialEq` by default |
| `interface` with methods / behaviour | `trait` (+ `impl` for the struct) | traits for behaviour you substitute (stores, clocks); structs for data |
| `class` with inheritance | composition + `trait` | no inheritance: shared behaviour becomes a trait with default methods |
| `type UserId = string` (alias) | `struct UserId(String)` newtype | aliases are interchangeable in TS; newtypes are not, which is the point. `#[serde(transparent)]` keeps the wire shape |
| `A & B` intersection | a struct holding both, or `#[serde(flatten)]` | |
| `Partial<T>`, `Pick<T,K>`, `Omit<T,K>` | a separate struct (`IssuePatch`) | there is no mapped-type equivalent; write the struct and a `From`/`apply` |
| `Record<ClosedKeys, V>` | enum + `match` | not a map; an array indexed by `as usize` trips the `indexing_slicing` gate lint; see snippet D |
| `Record<string, V>`, `{[k: string]: V}` | `BTreeMap` (sorted), `IndexMap` (insertion), `HashMap` (unordered) | choose by whether order is observable; see `json-fidelity.md` section 5 |
| `"x" in obj` | explicit set/enum membership | `in` also sees the prototype chain: `"constructor" in {urgent: 0}` is true. The worked example freezes this bug and the port rejects it |
| numeric `enum`, `as const` objects | `enum` with explicit discriminants + `as_str` / `FromStr` | numeric TS enums also have reverse mappings (`E[0]`); port only what callers use |
| `T[]` | `Vec<T>` (owned), `&[T]` (borrowed) | |
| `[A, B]` tuple | `(A, B)` | |
| generics `<T extends X>` | generics with trait bounds | conditional and mapped types have no equivalent: redesign per use |
| `Date` | `chrono::DateTime<Utc>` or `time::OffsetDateTime` | pick one project-wide; inject the clock (snippet F) |
| `Buffer` / `Uint8Array` | `Vec<u8>` / `&[u8]` | |
| `RegExp` | `regex` crate | its documentation lists no look-around or backreferences `[unverified: not re-checked]`: check the patterns you use |

## 2. Unknown, any, and casts

- `unknown` / `any` becomes `serde_json::Value` **only at the module boundary**, validated straight away with
  `serde_json::from_value::<T>` or hand-written checks. Never let `Value` flow through the module.
- **`x as T` in TypeScript checks nothing.** In the worked example `assignee: r.assignee as string | null |
  undefined` accepts `5`. The port validates; that is a behaviour change, so it is registered.
- Freeze the TS behaviour for garbage input too: the corpus is where unsound casts show up.

## 3. Absence, defaults, errors

| TypeScript | Rust |
|---|---|
| `T \| undefined`, `T \| null` | `Option<T>`; three-state `a?: T \| null` needs `Option<Option<T>>` (`json-fidelity.md`) |
| `a ?? b` | `a.unwrap_or(b)`: replaces only null/undefined, keeps `0` and `""` |
| `a \|\| b` | **not** `??`: `0 \|\| "x"` is `"x"`. Port as an explicit `if n == 0` (snippet E) |
| `a?.b?.c` | `a.as_ref().and_then(\|a\| a.b.as_ref()).and_then(\|b\| b.c.clone())` for an owned `Option`; `a.and_then(\|a\| a.b.as_ref())` returns a reference to a local and fails with E0515 `[verified: rustc 1.98.1]` (snippet E2) |
| `throw new TypeError(msg)` | an error enum variant; keep the message if any caller can see it |
| `try { } catch (e) { if (e instanceof X) }` | `?` plus `match` on the variant |
| `Promise` rejection | `Result` in the async return type |
| uncaught exception as bug | `panic!` only for bugs; user input never panics (index with `get`, parse with `?`) |

Library code uses a `thiserror` enum (snippet G). Reserve `anyhow` for binaries.

## 4. Async

| TypeScript | Rust |
|---|---|
| `async function f(): Promise<T>` | `async fn f() -> T` (or `Result<T, E>`) |
| `await` in a loop | sequential `.await`s |
| `Promise.all` | `futures::future::try_join_all`: fails fast on the first error `[verified: futures 0.3.34]` |
| `Promise.allSettled` | `futures::future::join_all` over `Result` futures `[verified: futures 0.3.34]` |
| callbacks / `EventEmitter` | channels, or a trait the caller implements |

Keep library code runtime-agnostic: no `tokio::spawn` inside it, take collaborators as trait parameters. A public
`async fn` in a trait trips rustc's `async_fn_in_trait` lint because callers cannot demand `Send` futures. Choose:
declare `fn get(&self, ..) -> impl Future<Output = T> + Send` (callers may spawn), or allow the lint with a reason
when the target is single-threaded and futures need not be `Send` (snippet C). Targets whose futures are not
`Send` cannot implement the first form.

## 5. Mutation and ownership

JavaScript objects are shared by reference; Rust values are moved or borrowed. Typical traps:

- `const a = b` aliases, and mutation through either is visible through both. In Rust decide who owns the value.
- `Array.prototype.sort()` mutates in place (checked) and `toSorted()` returns a copy. Port the signature to the
  intent: `fn sorted(&[T]) -> Vec<T>` or `fn sort_in_place(&mut [T])` (snippet H). Do not hide an in-place sort
  behind a `&[T]` by cloning silently.
- Clone at ownership boundaries, not to quiet the borrow checker (`anti-patterns.md`).
- Callbacks that mutate captured state are `FnMut`; shared mutable state is a design question, not an
  `Arc<Mutex<_>>` by reflex.
- `Date.now()` and `Math.random()` are hidden inputs: inject them so freeze and replay are deterministic.

## 6. Numbers, quickly

Counts and indices `usize`/`u32`; ids `i64`/`u64`/`Uuid` (ids above 2^53 are not safe in JS); money as integer
minor units or `rust_decimal`, never `f64`; measurements `f64`. For integer-to-integer conversions use
`i32::try_from` / `u8::try_from` instead of `as`; a float-to-int `as` needs a range check and a justified
`#[allow(clippy::cast_possible_truncation, reason = "...")]` (details and the `as` semantics are in
`json-fidelity.md` section 2).

## 7. Snippets (compiled)

Standalone: edition 2021 or later; the crate needs `serde` (with `derive`), `serde_json` and `thiserror`.

```rust
use serde::{Deserialize, Serialize};
use std::future::Future;

pub struct Issue { pub name: String, pub estimate: Option<f64> }

// A: discriminated union -> enum; switch + never-check -> exhaustive match
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Shape { Circle { r: f64 }, Square { side: f64 } }
pub fn area(s: &Shape) -> f64 {
    match s {
        Shape::Circle { r } => std::f64::consts::PI * r * r,
        Shape::Square { side } => side * side,
    }
}

// B: behaviour interface -> trait
pub trait Describe { fn describe(&self) -> String; }

// C: Promise-returning interface. Two honest choices:
#[allow(async_fn_in_trait, reason = "single-threaded targets: futures need not be Send")]
pub trait Store { async fn get(&self, id: u32) -> Option<String>; }
pub trait SendStore { fn get(&self, id: u32) -> impl Future<Output = Option<String>> + Send; }
// an impl of SendStore can be written with a plain `async fn get`.

// newtype ids: not interchangeable, same wire shape
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserId(pub String);

// D: Record<ClosedKeys, V> -> enum + match
#[derive(Debug, Clone, Copy)]
pub enum Level { Low, High }
pub fn weight(l: Level) -> u8 { match l { Level::Low => 1, Level::High => 9 } }

// Partial<T> -> separate struct; `??` -> unwrap_or
#[derive(Debug, Default, Deserialize)]
pub struct IssuePatch { pub name: Option<String>, pub estimate: Option<f64> }
impl Issue {
    pub fn estimate_or_zero(&self) -> f64 { self.estimate.unwrap_or(0.0) } // keeps 0
}

// E: `||` on falsy is not `??`
pub fn or_default(n: i64) -> i64 { if n == 0 { 7 } else { n } }

// E2: a?.b?.c on an owned Option
pub struct Leaf { pub c: Option<String> }
pub struct Mid { pub b: Option<Leaf> }
pub fn chain(a: Option<Mid>) -> Option<String> {
    a.as_ref().and_then(|a| a.b.as_ref()).and_then(|b| b.c.clone())
}

// F: inject the clock
pub trait Clock { fn now_ms(&self) -> i64; }

// G: exceptions -> Result; instanceof -> match
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found: {0}")] NotFound(String),
    #[error("invalid input: {0}")] Invalid(String),
    #[error(transparent)] Json(#[from] serde_json::Error),
}
pub fn classify(e: &ApiError) -> u16 {
    match e { ApiError::NotFound(_) => 404, ApiError::Invalid(_) | ApiError::Json(_) => 400 }
}

// H: sort() mutates; pick the signature that matches the intent
pub fn sorted(xs: &[i64]) -> Vec<i64> { let mut v = xs.to_vec(); v.sort(); v }
pub fn sort_in_place(xs: &mut [i64]) { xs.sort(); }
```
