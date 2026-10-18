# `dx translate`: verified behaviour and the fix list

`[verified: dioxus 0.7.10 (57d6794), macOS, 2026-10-18]`: fixture runs plus a read of the converter source at the
`v0.7.10` tag. Output may differ in other releases: re-run the drift check below after any `dx` upgrade.

## 0. Make sure `dx` is Dioxus

Preflight: `dx --version` must print `dioxus 0.7.x`. Some machines have another `dx` earlier on PATH: Deno ships
a `dx` alias for `deno x`, whose `--help` starts "Execute a binary from npm or jsr", and `dx --version` prints a
deno banner. If it does, run no other `dx` command (`dx translate` would ask `deno x` to fetch and run an npm
package named `translate`), and ask the user to put the Dioxus CLI first on PATH. Everything below assumes the
Dioxus CLI.

Never pre-approve `dx` with a wildcard (for example `dx translate:*`) in a permission list. Approve each
invocation, or only the exact `dx --version` probe.

## 1. Syntax (from `dx translate --help`)

| Flag | Meaning | Note |
|---|---|---|
| `-f, --file <FILE>` | read HTML from a file | mutually exclusive with `--raw` (the tool errors if both are given) |
| `-r, --raw <RAW>` | HTML given inline | help text says "Input file"; it is an inline string (verified) |
| *(neither)* | reads stdin when stdin is not a terminal | errors with "No input file, source, or stdin" otherwise |
| `-o, --output <FILE>` | write to a file | stdout if absent |
| `-c, --component` | wrap in `fn component() -> Element` and lift each `<svg>` into `mod icons { fn icon_N }` | help text says "Activate debug mode"; that is wrong |

Default mode prints the bare node tree (a leading blank line, 4-space indent), not a function. Wrap it yourself:
`fn Card() -> Element { rsx! { <output> } }`. Translate fragments (a card, a row, a nav), not a whole document:
a full `<html>` input yields `html { head { .. } body { .. } }`, which is not what a Dioxus app root renders.

## 2. What the converter does and does not do

Observed on fixtures (see the drift check) and confirmed in `rsx-rosetta`'s source.

| HTML input | Output | What you do |
|---|---|---|
| `<button disabled>` (any bare boolean attribute: `checked required readonly autofocus selected autoplay muted loop hidden`) | `disabled: "false"` | **Wrong meaning.** Rewrite to `disabled: true`. Cause: a valueless attribute defaults to the text `"false"` |
| `disabled="disabled"` | `disabled: "disabled"` | rewrite to `disabled: true` |
| `onclick="go()"` | `"onclick": "go()"` (a string attribute, not a handler) | rewrite as a real handler (syntax: `dioxus-07`, if installed; else the Dioxus 0.7 docs) |
| `<!-- comment -->` | dropped | nothing |
| `class="a  b"`, `id` | class tokens re-joined with single spaces; `class` and `id` appended, then **all attributes sorted alphabetically** | nothing (order is cosmetic) |
| `for`, `type`, `loop` | `r#for`, `r#type`, `r#loop` | nothing |
| `viewBox`, `stroke-width` | `view_box`, `stroke_width` | nothing |
| `aria-label` | `aria_label` | confirm it compiles; keep every aria/role/tabindex attribute |
| `data-state="open"`, unknown attributes | `"data-state": "open"` (string key) | nothing |
| `<custom-element foo="bar">` (hyphenated, unknown name) | `custom-element { "foo": "bar", .. }` (string-key attribute) | nothing |
| `<script>`, `<style>` | emitted as elements containing the text | move to the asset pipeline or the stylesheet |
| inline `style="..."` | kept as a string | prefer tokens/classes; keep only if the original needs it |
| `<img src="/assets/a.png">` | path kept as a string | route through Dioxus's asset mechanism (`dioxus-07`, if installed) |
| `--component` with `<svg>` | `icons::icon_0 {}` plus `mod icons` | fine |

### Known defect (0.7.10): `--component` emits unbalanced parentheses

The `--component` output ends the macro with `})` instead of `}` (twice when an svg is lifted), so it does not
parse: `error: unexpected closing delimiter: '}'`. The upstream source at the v0.7.10 tag writes
`out.push_str("    })\n}")`; upstream main wrote `"    }\n}"` on 2026-10-18. On a release without the fix, use
default mode and wrap by hand, or delete the stray `)`.

## 3. Syntax-only gate that needs no dependencies

Token-tree balance is checkable offline with a stub macro (it does not validate RSX, only that the output
parses). The source is fed to `rustc` on stdin and the metadata goes to a temporary directory that is deleted, so
nothing is written to the project. The input file must exist and be non-empty (`-s`), and the temporary directory
must exist (`-d`), or the chain stops before `rustc` and prints `SYNTAX FAILED`. For default-mode output (the
fragment is wrapped by hand; `f` is a `frag-<name>.rsx` file):

```sh
f=capture/<route>/<viewport>/frag-<name>.rsx
tmp=$(mktemp -d)
[ -s "$f" ] && [ -d "$tmp" ] && { printf 'macro_rules! rsx { ($($t:tt)*) => { () }; }\ntype Element = ();\nfn component() -> Element { rsx! {\n'
  cat "$f"; printf '\n} }\n'; } |
  rustc --edition 2021 --crate-type lib --crate-name syntax_gate -A warnings --emit=metadata --out-dir "$tmp" - && echo SYNTAX OK || echo SYNTAX FAILED
rm -rf "$tmp"
```

For `--component` output, which already holds `fn component()`, use the same command with no wrapper: replace the
block in braces with `{ printf 'macro_rules! rsx { ($($t:tt)*) => { () }; }\ntype Element = ();\n'; cat "$f"; }`.
A `SYNTAX OK` that appears together with a `cat:` error means the input was missing: the result is void.

`[verified: rustc 1.98.1]` under bash 5.3.15 and zsh 5.9.2:

| Input | Result |
|---|---|
| default-mode output | `SYNTAX OK` |
| `--component` output as 0.7.10 writes it (the control) | `error: unexpected closing delimiter`, `SYNTAX FAILED` |
| the same `--component` output with the stray `)` removed | `SYNTAX OK` |
| a missing file, default or `--component` variant | `SYNTAX FAILED`, no `SYNTAX OK` |
| an empty file, default or `--component` variant | `SYNTAX FAILED`, no `SYNTAX OK` |
| the unguarded form (no `-s`) with a missing file | `cat:` error **and** `SYNTAX OK` (the defect the guard removes) |

The project directory is unchanged afterwards. The real gate is `cargo check` in the project.

## 4. Translate first, refactor second (protocol)

1. Take a **sanitised** DOM fragment from capture (no scripts, no inline data, no tokens).
2. `dx translate --file capture/<route>/<viewport>/frag-<name>.html --output capture/<route>/<viewport>/frag-<name>.rsx`,
   one fragment per file (a route has several: `frag-card`, `frag-nav`, ...). The output stays under `capture/`,
   which the skill may edit; `.rsx` is not Rust until wrapped.
3. Wrap it into `src/<component>.rs` and get it compiling. Take the fragment from the **widest viewport's** DOM
   (the layout the other viewports collapse from); the narrower viewports' captures are for the diff, not for
   separate components. **Ask the user, then check in that state alone** ("raw translation", the agent does not
   commit): every later diff then shows exactly what the refactor changed.
4. Apply the fix list in order: booleans, handlers, script/style, asset URLs, then dynamic text -> props or
   signals, repeated rows -> a `for` over data (syntax: `dioxus-07`, if installed).
5. Keep utility class strings verbatim when the original's stylesheet is reused; do not re-derive CSS by hand.
6. Extract components where the same structure repeats with different text/props, or where a block owns its own
   state (open/closed). Then swap in equivalents from the Dioxus components library via `dioxus-design-system`
   (skip this step and note it in `clone-report.md` if that skill is not installed); its own approval flow governs
   any `dx components` command it asks for.
7. Re-run the diff gate (`visual-diff.md`) after each extraction, not once at the end.

A cheap pre-diff before screenshots: compare the tag-and-class outline of the original capture with the DOM of
the running port `[unverified: method only]`.

## 5. Drift check after a `dx` upgrade

Translate this fixture and confirm the quirks above hold (or update the fix list):

```html
<div class="card w-96" id="c1" data-state="open">
  <button type="button" disabled onclick="go()" style="color: red">Save</button>
  <input type="checkbox" checked required>
  <!-- note -->
  <svg viewBox="0 0 10 10"><path d="M0 0L10 10" stroke-width="2"/></svg>
</div>
```

Expected on 0.7.10: `disabled: "false"`, `"onclick": "go()"`, `checked: "false"`, `required: "false"`, no comment,
`view_box`, `stroke_width`. Anything else means the converter changed.
