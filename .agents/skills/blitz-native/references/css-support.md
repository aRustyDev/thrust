# What CSS does the native renderer support, and how to check

This skill performs no network access. The data sources are public; open them in a browser or have the user
download the files, then apply the method. Retrieved 2026-10-18.

## Evidence ladder (strongest first)

1. **A minimal repro on the project's pinned version.** Nothing else answers "does it work for me".
   `[unverified: no native window was run for this skill]`
2. **WPT results** (Web Platform Tests run on Blitz per commit): per-spec pass rates, with the caveats below.
3. **The upstream roadmap** (`DioxusLabs/blitz` issue #119): states what each milestone added.
4. **Inference** from the architecture (for example no JavaScript engine). Label it `inferred`.

## WPT results: where and how

- Repository: `DioxusLabs/blitz-wpt-results` ("Archive of WPT test results for Blitz"; no licence declared).
  The `summary/` directory is a compact time series. The Blitz README links `blitz.is/status/css` as its status
  page and the summary README names `blitz.is/status/wpt` for the charts.
- `summary/runs.json`: `{"runs":[{date, wpt_revision, product_revision, commit_message}, ...]}`, one entry per
  tested Blitz commit, sorted by date. `summary/areas/css.json` and `summary/areas/css/<spec>.json` (one file per
  WPT folder, for example `css-flexbox.json`) hold a `scores` array **index-aligned with `runs`**.
- Each non-null entry is `[total_tests, servo_style_score, total_subtests, subtests_passed]`.
  Pass rate = `scores[3] / scores[2]`. `SKIP` tests are stripped before counting.
- On downloaded files (offline). Latest value for one spec:
  `jq '[.scores[] | select(. != null)] | last | {tests: .[0], subtests: .[2], passed: .[3], pct: (.[3]/.[2]*100)}' css-flexbox.json`
- Which Blitz commit does an index describe? Project to the two safe fields (the commit message is upstream
  free text, so leave it out):
  `jq '.runs[-1] | {date, product_revision}' runs.json` (latest) or `jq '.runs[395] | {date, product_revision}' runs.json`.
- The snapshot in this skill is `wpt-snapshot.md` (its run date is inside the file). After 2027-01-15, report
  it as stale and ask the user for the current `summary/` files.

### Caveats that change the answer

- **WPT is conformance, not usability.** Many subtests exercise corners no app touches; a 60% area can be fine
  for an app and a 90% area can still miss the one property you need. Test the property.
- **The denominator moved.** Between run 395 (2025-10-22) and run 1195 (2026-10-17) the CSS suite went from
  24,175 to 34,620 tests and from 33,405 to 198,605 subtests, and the WPT revision changed (`42a9dfcc6` then,
  `cd1d8aa5b` now). The sources do not say why. Percentages from different dates are **not comparable**, and
  an area with a tiny denominator (for example `css-transitions`, 6 subtests in 2025) says almost nothing.
- **`main` is not 0.7.x.** A Dioxus 0.7.x app requires blitz 0.2.x (blitz-dom `^0.2.4`, published 2025-10-22).
  The nearest recorded run to that date is index 395 (2025-10-22, Blitz commit `5f932bfc68`, WPT revision
  `42a9dfcc6`).

## Primary for 0.7 questions: run 395 (2025-10-22, approximately blitz-dom 0.2.4)

Whole CSS suite: 33,405 subtests, 12,324 passed (**36.9%**). Selected areas (subtests run, passed, pass %):

| WPT area | Subtests | Passed | Pass % |
|---|---:|---:|---:|
| css-variables | 186 | 179 | 96.2 |
| css-ui | 971 | 806 | 83.0 |
| css-fonts | 317 | 261 | 82.3 |
| css-lists | 166 | 128 | 77.1 |
| css-text-decor | 272 | 202 | 74.3 |
| css-text | 1,556 | 761 | 48.9 |
| css-transforms | 778 | 371 | 47.7 |
| filter-effects | 297 | 121 | 40.7 |
| css-flexbox | 3,430 | 1,392 | 40.6 |
| css-masking | 359 | 110 | 30.6 |
| css-overflow | 584 | 159 | 27.2 |
| css-grid | 4,747 | 1,251 | 26.4 |
| css-borders | 55 | 14 | 25.5 |
| css-backgrounds | 691 | 152 | 22.0 |
| css-tables | 540 | 115 | 21.3 |
| css-writing-modes | 1,083 | 223 | 20.6 |
| css-align | 1,678 | 321 | 19.1 |
| css-sizing | 1,568 | 293 | 18.7 |
| selectors | 219 | 40 | 18.3 |
| css-position | 269 | 45 | 16.7 |

(`css-env` had no data at that run.) Use the table for order of magnitude only: low scores in 2025 partly
reflect which tests were running, not only which features existed.

## Current `main` (the 0.3 line; full table in `wpt-snapshot.md`)

Run 1195 of 1196 (2026-10-17, commit `fb8d6c15e9`, WPT revision `cd1d8aa5b`): whole CSS suite 198,605
subtests, 118,644 passed (**59.7%**). Highlights: css-flexbox 86.2%, css-variables 81.5%, css-grid 78.0%,
css-backgrounds 73.5%, css-text 71.5%, css-position 66.1%, css-transforms 65.7%, css-overflow 38.1%,
css-multicol 39.6%, css-writing-modes 43.8%, css-shadow 6.0%. Stale after 2027-01-15.

## Roadmap facts (`DioxusLabs/blitz` issue #119, open, updated 2026-10-14)

- **0.2 (Oct 2025, the line 0.7.x requires):** incremental box-tree construction, CSS animations and
  transitions, wgpu 26, WPT on every PR. Earlier (0.1): `calc()`, `object-fit` / `object-position`,
  background size, position and repeat, WGPU texture rendering (`<canvas>`), grid named lines and areas.
- **0.3 milestone, items checked off (the absence of these from 0.7.x is inferred from milestone placement,
  not tested):** nested documents and `<iframe>`, text selection and
  `user-select`, `pointer-events`, `touch-action`, overlay scrollbars and `scrollbar-width` / `scrollbar-color`,
  `scroll-behavior`, custom widget API, `<details>` / `<summary>`, `clip-path`, `mask`, `filter`,
  `backdrop-filter`, floats, `border-collapse: collapse`, sizing keywords (`min-content`, `max-content`,
  `fit-content`, `stretch`), `position: static` and `position: fixed`, `vertical-align`, box-level
  `direction: rtl`, many event types (pointer, touch, focus, scroll, wheel, extra mouse events). A 0.7.x app
  should **not** assume these `[inferred from milestone placement; not run]`. (`<details>` / `<summary>`
  also appears under the roadmap's "Archived Roadmap Items" section; the milestone placement above is the
  issue's 0.3 list.)
- **0.3 milestone, items still unchecked when read:** `<body>` and `<html>` events, drag and drop,
  `text-overflow`, Android app-icon support in the CLI.
- **1.0, open:** `position: sticky`; `min-content` / `max-content` / `fit-content()` in `min-` and `max-`
  size styles; Flow layout (block/float) bug fixing; table layout; `contain`; `line-clamp`;
  `text-wrap-style`; reliable font fallback, `font-display`, `unicode-range`; `background-clip: text`;
  text shadow; `mix-blend-mode`; Shadow DOM and custom elements; `<picture>` / `srcset`, `<select>`
  dropdown, range input, colour picker; animation events; devtools; richer accessibility tree. Clipboard
  events and composition / IME are listed with a question mark upstream (undecided).
- **Backlog:** JavaScript support, 3D transforms, first-party stylable SVG, real table layout (currently
  emulated with grid), subgrid, anchor positioning, multi-column, `writing-mode` and vertical text, ruby.

Cross-check with the WPT data: low scores in `css-multicol`, `css-writing-modes`, `css-contain` and
`css-tables` are consistent with the roadmap's open items; `css-shadow` (6%) plausibly reflects the missing
Shadow DOM `[inferred]`.

## Practical rules for app authors

- Prefer flexbox, grid, block layout, CSS variables, `calc()`, colour, backgrounds, borders, transitions.
- Avoid for now (per roadmap, verify in a repro): sticky headers, multi-column text, vertical writing, real
  `<table>` sizing subtleties, text shadow, blend modes, `contain`, and anything needing JavaScript.
- Test with the exact `dioxus-native` version that ships; re-run the check on every Dioxus upgrade.
