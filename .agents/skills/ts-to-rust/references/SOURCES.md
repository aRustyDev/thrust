# Sources for ts-to-rust

| Source | Supports | Retrieved | Licence |
|---|---|---|---|
| https://github.com/Xellos1010/sdlc-visual-workflow-workspace: `.claude/skills/rust-porter/SKILL.md`, `.claude/skills/ts-react-rust-analyzer/SKILL.md` | the keep / adapt / defer / drop vocabulary and the definitions of the four decisions (wording adapted in `references/decision-table.md`); a ledger row per slice; placeholders that fail loudly | 2026-10-18 | MIT |
| https://github.com/Xellos1010/sdlc-visual-workflow-workspace: `.claude/skills/rust-migration-verifier/SKILL.md` | an independent verification step with a pass/fail evidence list | 2026-10-18 | MIT |
| https://github.com/realadarsh/cpp-to-rust-skill: `SKILL.md` | the workflow skeleton: freeze behaviour with tests before rewriting, a parity harness as the gate, vertical slices, completion gates | 2026-10-18 | MIT |

The decision-table wording is adapted from the MIT-licensed `rust-porter` and `ts-react-rust-analyzer` skills, and
a few short phrases in it are kept verbatim ("no framework lock-in", "direct port", "new Rust model", "compile-safe
stub"). Upstream notice for those two skills: "MIT License. Copyright (c) 2026 Flagship Foundry contributors".
Nothing was taken verbatim from the other two sources, which informed the workflow only. The rest of the skill is
original.

Verified with: Node v26.9.0, rustc 1.98.1, cargo 1.98.1, clippy 0.1.98, serde 1.0.229, serde_json 1.0.154 (and 1.0.143 for float text), chrono 0.4.45, uuid 1.28.0, thiserror 2.0.21, futures 0.3.34, python3 3.14.7, uv 0.12.18 and grep (BSD grep, GNU compatible) 2.6.0-FreeBSD, on 2026-10-18.
