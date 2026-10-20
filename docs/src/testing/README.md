# Testing

This chapter is the map of how thrust is tested: which technique catches which kind of defect, which tests a change
needs, and the loop an agent follows to write them test-first. It is written for agents first and people second:
every page names exact paths and commands, and every technique says whether it is real today.

It does **not** teach how to write a given kind of test. That is the job of the skills under `.agents/skills/`
(`rust-test-logic`, `rust-test-fuzz`, `rust-test-mutation`, `rust-test-app`, `ts-to-rust`); each technique page
links the one that owns it.

## Read order

| You are about to... | Read |
|---|---|
| change code and need to know which tests to write | [Which tests a change needs](by-change.md), then [The TDD loop](tdd-loop.md) |
| write one kind of test | its page under [Techniques](techniques/README.md), then the owning skill |
| run a suite or interpret an exit code | [Running the suites](running.md) |
| judge whether a passing test proves anything | [Evidence and controls](evidence-and-controls.md) |
| decide what to test next across the project | [Defect matrix](defect-matrix.md) and [Status, gaps and drift](status.md) |
| find out why a rule exists | [Rules](rules.md) |

## Status legend

Every technique page starts with one of these. A technique's status lives only on its own page; other pages name the
technique and link to it.

| Status | Meaning |
|---|---|
| **live** | Exercised in this repo today by a test, script or gate you can run. |
| **template-only** | A skill teaches it and the config routes it, but nothing in the repo uses it yet. Your change may be the first. |
| **planned** | Required by an ADR, `TRACEABILITY.md` or a gap entry, with no tooling or skill yet. The page links the owner and stops there. |

Command blocks are tagged the same way the skills tag them: `[verified <date> <sha>]` (run, and it did what the
page says), `[unverified: not run]`, or `[derived]` (read from the source, not executed).

## Where each fact lives

A fact written in two places drifts. Each kind of fact has one owner; everything else links to it.

| Fact | Owner | This chapter |
|---|---|---|
| The binding testing rules 1-9 | `AGENTS.md` (§Testing) | includes them verbatim in [Rules](rules.md); elsewhere cites "Rule N" |
| How to write a test of kind X | the owning skill in `.agents/skills/` | links to it; never pastes its templates |
| Profiles, filters, timeouts, retries | `.config/nextest.toml`, `.cargo/mutants.toml` | [Running](running.md) explains them |
| What a script does and its arguments | the script's header comment | quoted in [Running](running.md) |
| Decisions and the target state | ADRs 0120-0123 in `.agents/plans/contrail/adrs/test/` | one line and a path each, in [Strategy](strategy.md) |
| Which test proves which requirement | `.agents/plans/contrail/TRACEABILITY.md` ("Test home" column) | [The TDD loop](tdd-loop.md) says how to look it up |
| A technique's status | that technique's page | |
| What is measured today, gaps, drift | [Status, gaps and drift](status.md) | |
| Testing terms | [Glossary](glossary.md) | |

If this chapter and a **rule** in `AGENTS.md` disagree, the rule wins. If they disagree about a **fact** (what a
command selects, where tests live), re-measure: the measured answer wins, and `AGENTS.md` prose known to be stale is
listed as drift in [Status](status.md). Record any new disagreement there.

Repository files are cited as root-relative code spans (`scripts/contract.sh`), not links: the book only publishes
`docs/src/`, so a link to a file outside it would break.
