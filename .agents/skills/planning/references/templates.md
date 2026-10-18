# Starting points

Skeletons for the two documents created most often. Fill them from the project; delete any section that has nothing to say rather than leaving the placeholder.

## ADR (MADR-style)

File: `docs/src/dev/adrs/<topic>/NNNN-<short-title>.md`, linked from `docs/src/SUMMARY.md`. Field and section names follow the MADR project's full template (see `SOURCES.md`); the optional sections are shown because they are the ones used most.

```markdown
---
status: "proposed | accepted | rejected | deprecated | superseded by ADR-NNNN"
date: YYYY-MM-DD
---

# <short title, representative of the problem solved and the solution found>

## Context and Problem Statement

<two or three sentences; link the plan questions or research this came from>

## Considered Options

* <option 1>
* <option 2>

## Decision Outcome

Chosen option: "<option 1>", because <justification>.

### Consequences

* Good, because <...>
* Bad, because <...>

### Confirmation

<how compliance is checked: a gate command, a test, a review step>
```

Accepted ADRs are immutable (`graduation.md`).

## Plan README

File: `.agents/plans/<plan-name>/README.md`.

```markdown
# <plan name>

<one paragraph: what this plan is and what done looks like>

## Map

| Path | Contents |
|---|---|
| `PLAN.md` | the sequence of work and its current status |
| `questions/` | open questions (numbered globally; gaps are expected) |

## Status (<date>)

<short table of what is done, held or blocked, each with its open items>
```

List only paths that exist. Numbering gaps are explained here rather than renumbered.
