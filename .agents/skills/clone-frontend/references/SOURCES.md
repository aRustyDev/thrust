# Sources for clone-frontend

| Source | Supports | Retrieved | Licence |
|---|---|---|---|
| https://github.com/DioxusLabs/dioxus, `packages/cli/src/cli/translate.rs` at tag `v0.7.10` and the default branch | `dx translate` flags, `--file`/`--raw` exclusivity, stdin input; the unbalanced `)` written by `--component` at `v0.7.10` (absent on the default branch at retrieval) | 2026-10-18 | Apache-2.0 (GitHub licence API) |
| https://github.com/DioxusLabs/dioxus, `packages/rsx-rosetta/src/lib.rs` at tag `v0.7.10` | why bare boolean attributes become `"false"`; comments dropped; attributes sorted; custom-element and svg handling | 2026-10-18 | Apache-2.0 |
| https://github.com/DioxusLabs/dioxus/releases | `v0.7.10` published 2026-07-30; `v0.8.0-alpha.3` published 2026-10-16 (pre-release); basis for the re-check-on-upgrade rule | 2026-10-18 | Apache-2.0 |
| https://github.com/DioxusLabs/components, README | the library describes itself as "a shadcn style component library", so mapping is shadcn-style, not one-to-one | 2026-10-18 | Apache-2.0 (GitHub licence API) |

No text is adapted from these sources; they support facts only.

Verified with: dioxus 0.7.10 (57d6794) `dx translate` fixture runs and `dx serve` argument parsing, rustc 1.98.1, uv 0.12.18, python3 3.14.7, node v26.9.0, bash 5.3.15, zsh 5.9.2, grep (BSD grep, GNU compatible) 2.6.0-FreeBSD and macOS `sips` (independent PNG encoder, decoder cross-check) on 2026-10-18; Deno 2.9.7's `dx` alias (`deno x`) was observed shadowing the Dioxus CLI.
