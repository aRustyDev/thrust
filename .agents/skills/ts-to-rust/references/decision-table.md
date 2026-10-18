# Scope and the keep / adapt / defer / drop decision (stage 1)

Port one bounded module at a time. Stage 1 produces a decision row per slice before any fixture or Rust exists.

## 1. Choosing the slice

- One module, one exported surface, one owner. If the exported surface needs more than a few hundred lines of
  TypeScript to describe, split it.
- Prefer **leaf slices first**: ones whose dependencies are already ported or are plain data. Each slice must be
  releasable on its own.
- Put I/O behind an interface (a trait on the Rust side) so the slice can be frozen and replayed without a
  network or filesystem.
- List callers. Behaviour no caller depends on is a candidate to drop; behaviour one caller depends on is part of
  the contract even if it looks like a bug.

## 2. The four decisions

Definitions adapted from the MIT-licensed `rust-porter` and `ts-react-rust-analyzer` skills, with a few short
phrases kept verbatim (copyright notice in `SOURCES.md`).

| Decision | Use when | Action | Evidence before it counts as done |
|---|---|---|---|
| **keep** | pure data, parsing or utility code with no framework lock-in; the behaviour is the contract | direct port: same logic, typed structs and enums | every fixture passes with **no** exception rows |
| **adapt** | observable behaviour must survive, but the mechanism cannot carry over (prototype tricks, `localeCompare`, framework hooks, Node APIs, unsound casts) | new Rust model, same observable behaviour | fixtures pass; each deliberate difference is a reasoned exception row |
| **defer** | blocked on another slice or an open decision, or the behaviour is not yet frozen | compile-safe stub that fails loudly (`anti-patterns.md` section 3), ledger row with blocker and owner | the stub cannot silently succeed; the ledger names what unblocks it |
| **drop** | an explicit non-goal: dead code, retired route, behaviour judged wrong | delete | a recorded rationale in the project's decision log; callers checked |

Defaults: if unsure between keep and adapt, choose adapt and write down why. If unsure between adapt and drop,
**do not drop**: defer until a caller list proves it dead.

## 3. Risk signals that push a slice from keep toward adapt

Framework coupling (React hooks, an HTTP framework's request object); dynamic features (prototype mutation,
`Proxy`, `eval`, dynamic `import`); hidden inputs (`Date.now()`, `Math.random()`, locale, timezone, environment
variables); `localeCompare` or `Intl`; numbers above 2^53 or money in floats; concurrency whose ordering is
observable; module-level mutable state; unchecked `as` casts and `any`. Each is a row in the fidelity pages and a
corpus case in the freeze.

## 4. Per-slice template

| Field | Content |
|---|---|
| slice | name and exported functions |
| source | file(s) and line count |
| callers | who depends on it |
| I/O | what it touches, and the trait that will hide it |
| risk | low / medium / high with the signals above |
| decision | keep / adapt / defer / drop, with the one-line reason |
| fixtures planned | corpus themes (`freeze-and-oracle.md` section 1) |
| target | Rust module path |

## 5. Ledger (`PORT-LEDGER.md` in the target crate; one row per slice, updated as it moves)

| slice | decision | stage reached | fixtures | control result | exceptions | gate | blocker / owner |
|---|---|---|---|---|---|---|---|
| ... | adapt | frozen / redesigned / ported / verified / gated | N | mutated fixture and port mutants failed as expected | M reasoned | clippy `--lib` clean | (defer only) |

`worked-example.md` fills one in.

## 6. What not to port

Build configuration, test utilities, framework glue and generated code are rarely behaviour. Treat them as drop
unless a caller or the freeze proves otherwise, and say so in the ledger.
