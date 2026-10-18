# WPT snapshot: Blitz `main`, run date 2026-10-17

**Dated data.** If today is after 2027-01-15 (90 days after the run date), report this snapshot as stale and
ask the user for the current `summary/` files rather than quoting it. It is Blitz `main` (the 0.3 line), run
index 1195 of 1196, product revision `fb8d6c15e9ce062b3b41e71cd48988f0093506cc`, run date
2026-10-17T13:08:25Z, WPT revision `cd1d8aa5b`. Source: `DioxusLabs/blitz-wpt-results` `summary/`
(retrieved 2026-10-18; no licence declared by the repository).

A Dioxus 0.7.x app uses Blitz 0.2, not this line: use the 0.7-era table in `css-support.md` for 0.7
questions. Percentages here and there are **not comparable**: the sets of tests that ran differ (see the
numbers in `css-support.md`).

Whole CSS suite: 34,620 tests, 198,605 subtests, 118,644 passed (**59.7%**). Pass rate is subtests passed
divided by subtests run, per WPT spec area, sorted high to low. Selection criterion: 48 of the 89 `css/`
area files, hand-picked for layout, box model, text, colour, effects, selectors and animation; the other 41
(for example `css-anchor-position`, `css-view-transitions`, `css-ruby`) were not read.

| WPT area | Subtests | Passed | Pass % |
|---|---:|---:|---:|
| css-style-attr | 16 | 15 | 93.8 |
| css-syntax | 442 | 412 | 93.2 |
| css-align | 5,373 | 5,002 | 93.1 |
| css-env | 53 | 48 | 90.6 |
| css-color | 13,002 | 11,435 | 87.9 |
| css-images | 4,063 | 3,553 | 87.4 |
| css-flexbox | 5,820 | 5,017 | 86.2 |
| css-variables | 741 | 604 | 81.5 |
| css-conditional | 2,307 | 1,870 | 81.1 |
| css-nesting | 149 | 120 | 80.5 |
| css-logical | 1,341 | 1,075 | 80.2 |
| css-ui | 3,350 | 2,647 | 79.0 |
| CSS2 | 8,820 | 6,926 | 78.5 |
| css-grid | 17,571 | 13,698 | 78.0 |
| css-content | 322 | 248 | 77.0 |
| css-viewport | 421 | 314 | 74.6 |
| css-fonts | 8,894 | 6,623 | 74.5 |
| css-backgrounds | 6,962 | 5,115 | 73.5 |
| css-cascade | 1,076 | 770 | 71.6 |
| css-text | 6,286 | 4,496 | 71.5 |
| css-lists | 1,160 | 807 | 69.6 |
| css-text-decor | 1,694 | 1,119 | 66.1 |
| css-position | 1,734 | 1,146 | 66.1 |
| css-transitions | 1,402 | 925 | 66.0 |
| css-transforms | 6,334 | 4,161 | 65.7 |
| css-borders | 1,771 | 1,115 | 63.0 |
| css-box | 1,132 | 704 | 62.2 |
| css-sizing | 6,492 | 4,007 | 61.7 |
| css-scrollbars | 115 | 68 | 59.1 |
| css-animations | 1,478 | 816 | 55.2 |
| css-values | 10,519 | 5,774 | 54.9 |
| css-display | 667 | 365 | 54.7 |
| css-masking | 6,894 | 3,747 | 54.4 |
| filter-effects | 3,310 | 1,763 | 53.3 |
| css-tables | 1,118 | 594 | 53.1 |
| selectors | 5,154 | 2,476 | 48.0 |
| css-contain | 974 | 465 | 47.7 |
| css-writing-modes | 1,471 | 645 | 43.8 |
| css-inline | 1,220 | 486 | 39.8 |
| css-multicol | 2,244 | 888 | 39.6 |
| css-overflow | 1,750 | 667 | 38.1 |
| css-counter-styles | 402 | 82 | 20.4 |
| css-scroll-snap | 767 | 156 | 20.3 |
| css-pseudo | 1,096 | 192 | 17.5 |
| css-break | 1,931 | 319 | 16.5 |
| mediaqueries | 456 | 49 | 10.7 |
| css-shapes | 7,759 | 717 | 9.2 |
| css-shadow | 400 | 24 | 6.0 |

A passing subtest does not prove a feature works: tests for features Blitz does not implement can pass
by only parsing the property (for example in `filter-effects` and `css-masking`) `[inferred]`.

## Maintainer note: regenerating

Download `summary/runs.json` and the area files named above from `blitz-wpt-results`, compute
`[.scores[] | select(. != null)] | last` per area (columns `[tests, score, subtests, passed]`), and replace
this file under the same name with the new run date, product revision and WPT revision. Update the stale
date in the text above and in `SKILL.md`.
