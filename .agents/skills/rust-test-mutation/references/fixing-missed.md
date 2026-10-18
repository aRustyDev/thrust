# Fixing a MISSED mutant

Tags as in `commands-and-results.md`: `[re-run]` reproduced with cargo-mutants 27.1.0, `[zip]` recorded
by the source's author, `[derived]` from the cargo-mutants book read 2026-10-18. This file writes no test
code: the property, model and example tests that kill mutants belong to `rust-test-logic`.

## Procedure

1. Read the line: `path:line:col: replace Type::method -> bool with true` `[re-run]`. It names the
   function and what it was replaced with.
2. Ask what observable behaviour would break if the code really did that.
3. Assert the **correct behaviour through the public API**, at the right level.
4. Prefer strengthening a property or model invariant so a whole class is covered. In the reference
   workspace the two `KvStore::is_empty` mutants (`-> bool with true`, `with false`) were killed by an
   `is_empty` assertion: removing only the state-machine invariant gave 18 caught, removing only the
   lockstep assertion gave 18 caught, removing both gave 2 MISSED and exit 2. Either assertion alone
   suffices; one invariant per class is the point `[re-run]`.
5. Re-run the file with `--iterate` until clean, then once without it (`--iterate` is a heuristic).
6. Skip only an equivalent mutant, or code that is genuinely not worth testing, and say why in the code.

**Never** write a test that targets the mutant (asserting the mutated value, or a test named after it).
**Never** skip or exclude to make the count go down. **Never** weaken an existing test. A fix you did not
see fail first is unproven: the control is the mutant itself, so after the fix the same line must move
from `missed.txt` to `caught.txt`.

## Reading the line

Common replacements `[derived]` and what a missed one usually means:

| Mutant | Usually means | Assert, through the public API |
|---|---|---|
| `replace f -> bool with true` and `with false` | the result is never checked on one branch | an input that makes it true and one that makes it false |
| `replace f -> T with Default::default()`, `0`, `1`, `String::new()`, `None`, `Ok(..)` | the return value is ignored or only checked loosely | the exact value for a non-trivial input |
| `replace f with ()` | a side effect is never observed | the state or output after the call |
| `replace + with -`, `< with >`, `== with !=` | the operator or boundary is untested | values at, just below and just above the boundary |
| `delete match arm ..` | the arm is never selected, or its result is unchecked | an input that selects that arm |
| `delete !` or `delete -` | the negation is untested | a case where the sign or the negation matters |

A function that returns `Result` gets `Err(..)` mutants only when `error_values` is configured
(`examples/mutants.toml`) `[derived]`: add it to find untested error paths.

## Equivalent mutants

A mutant is equivalent when no reachable state can tell it from the original: for example replacing a
capacity estimate passed to `Vec::with_capacity` with `0` changes speed, not behaviour. Prove it by
reasoning about reachable states, not by failing to think of a test. If you cannot prove it, it is a
real gap: write the test. An unproven "equivalent" is how a count gets lowered.

## Skipping, and its cost

| Case | Tool |
|---|---|
| One function, equivalent or untestable | `#[mutants::skip]` with a comment saying why |
| A class (for example `impl Debug`) | `exclude_re = ["impl Debug"]` in `.cargo/mutants.toml` `[derived]`; the regex matches the whole mutant name as `--list` prints it |
| Whole files (harness, opt-in e2e crate) | `exclude_globs` in the same file `[re-run]`; control in `interactions-and-ci.md` |
| Platform-specific code that `cfg` hides on this OS | skip with a comment; `cfg` is not evaluated |

`#[mutants::skip]` needs the `mutants` crate as a **regular** dependency, not a dev-dependency,
because the attribute sits on non-test code `[derived]`. The zip pins `mutants = "0.0.3"` `[zip]`; the
book asks for 0.0.3 or later and crates.io listed 0.0.4 as the newest on 2026-10-18 `[derived]`. A
caret requirement on a 0.0.x version matches only that exact patch, so pick the version deliberately
`[derived]`. The trade-off:
it adds a dependency to the crate that ships, tiny and with no runtime effect `[derived]`, but still one
more entry in its manifest. Check the repo's dependency guard or policy first: if it restricts what a
crate may depend on, the attribute may be refused. Then use `exclude_re` or `exclude_globs`, which
change only configuration. Prefer them anyway for a whole class; keep the attribute for one function
whose reason deserves to sit beside the code. Preview the combined effect of all three with `--list`
`[derived]`.

## Not a replacement for a hand-wired mutant

A hand-wired mutant that breaks exactly one rule remains the right tool to prove that one specific
assertion can fail: it is the must-fail control for a named check. cargo-mutants finds the assertions
nobody wrote. They are complementary, not interchangeable: keep the targeted control for the rules you
care about, and run cargo-mutants to find what you did not think to guard.
