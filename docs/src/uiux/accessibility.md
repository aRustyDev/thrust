# Contrast and colour blindness

The [palette](palette.md) is made of earth tones, and two things follow from that. Their low chroma means lightness has
to carry the contrast, and ochre, russet and umber can blur together for colour-blind users. This page states the rules
the palette is held to and what the checks measure. The measurements are `[verified 2026-10-19, draft palette]`, and
re-running the checker after any change to `.config/branding/colors.css` replaces them.

## The rules

1. **Text meets WCAG 2 contrast.** 4.5:1 for text, and 3:1 for a non-text marker such as the accent. The pairs that
   are checked are a fixed list in `.config/branding/check.py`: body, muted and link text on the page and on panels,
   text and links on the highlights, the accent as text and as a marker, text on an accent fill (the book does not
   draw one yet), inline code on mdBook's own code backgrounds, and each status colour as title text. They are
   checked in both themes.
2. **Status never relies on hue alone.** Every status needs a second cue, an icon or a label. The checker reports
   which status pairs colour alone cannot separate, so you know where the second cue is not optional.

## What the book does about rule 2

mdBook draws each alert with a coloured bar and a title that holds an icon and the kind's name. Each of the five kinds
(Note, Tip, Important, Warning, Caution) has an SVG icon and a text title in the generated page `[verified 2026-10-19,
mdBook 0.5.4]`. That was read from a scratch page with one alert of each kind, because the book itself uses none yet.
So the book's alerts never depend on colour alone. Anything else that shows a status must do the same.

## Measured: contrast

All 41 checked pairs pass (20 on the light theme, 21 on the dark one). Body text is 14.5:1 on the light theme and
17.1:1 on the dark one. These are the four pairs closest to their limit, all on the light theme. The dark theme's
closest is `status-important` at 5.68:1.

| Theme | Pair | Ratio | Needs |
|---|---|---|---|
| Light | `status-success` title text on `bg` | 4.64:1 | 4.5:1 |
| Light | `status-warning` title text on `bg` | 4.82:1 | 4.5:1 |
| Light | `accent` as a marker on `bg` | 3.36:1 | 3:1 |
| Light | `accent-text` on `surface-2` | 5.50:1 | 4.5:1 |

The earth tones pass because the steps differ a lot in lightness. A change that nudges a role toward the middle of
its ramp is what the check is there to catch.

Three cases the checker covers because a review found them:

- **Inline code.** mdBook draws it on its own code-block background, not on the palette's `surface-2`. The accent text
  on those backgrounds is 7.17:1 (light), 9.05:1 (coal and navy) and 9.08:1 (ayu). They are in the checker as literal
  pairs, because the palette does not set those backgrounds.
- **The search highlight.** mdBook's own highlight colours under the brand's light text on the dark themes were close
  to unreadable: 1.40:1 on navy and 1.65:1 on ayu. The palette now sets the highlight (`mark`), and the checker holds
  the text on it to 4.5:1: 11.11:1 on the light theme and 9.74:1 on the dark one.
- **The footnote flash.** Unless the reader prefers reduced motion, mdBook flashes `--footnote-highlight` behind a
  footnote reference for 0.8 seconds, with link-coloured text on it. Mapped to the accent, that was 2.05:1 on the light theme and 1.42:1 on the dark one. It
  now takes `mark` too, and the checker holds links on it to 4.5:1: 5.27:1 and 6.38:1.

## Measured: colour blindness

The checker simulates protanopia, deuteranopia and tritanopia (Machado, 2009, at full severity) and measures how far
apart each pair of status colours stays, as CIEDE2000, from the unrounded simulated colours. It takes the worst case of
the four views. Below 10 it says the pair is not told apart by colour alone. The 10 is a judgement, not a standard.

- **One pair is below it.** On the light theme `warning` and `danger` are 9.6 apart under the tritan model. Ochre and
  russet sit close together: three settings of `danger` were tried, and each gave up something. With `russet-800`,
  `danger` and `important` were 8.2 apart. With `russet-700`, the one kept, `warning` and `danger` are 9.6 apart.
  With `russet-500` both pairs clear 10 (10.7 and 27.3), but `danger` as title text fails contrast at 3.39:1. Both
  `warning` and `danger` keep their icons and titles in the book.
- **Everything else is at 10.7 or more**, in both themes.
- **The check found a real fault in the first draft.** `success` and `important` were 1.3 apart on the light theme and
  1.5 on the dark one, nearly identical to a deuteranope or a protanope: a green and a brown of nearly equal
  lightness. On the light theme they were `terre-verte-700` and `umber-700`, and on the dark theme `terre-verte-300`
  and `umber-300`. Moving them apart in lightness fixed it.

## Running the checks

```sh
uv run .config/branding/check.py     # [verified 2026-10-19, draft palette: exit 0]
```

`python3` works too, since the script uses only the standard library. Exit 0 means every contrast pair passes. Exit 1
means one does not, or a token cannot be resolved. Exit 2 means a self-test failed, and exit 3 that the token file
could not be read (it is missing, it is not UTF-8, or `--css` has no path). Each was seen: with a pale body-text
colour, with a missing file, a file that is not UTF-8 and a bare `--css`, and with a deliberately wrong luminance
constant in a scratch copy. The colour-blindness section is a
report: a flagged pair does not change the exit status.

The self-tests run first and stop the run if they fail. They check known answers (black on white is 21:1, `#767676` on
white passes 4.5:1 and `#777777` fails it, four CIEDE2000 reference pairs, a pair of hues across the 0/360 boundary,
and that each simulation leaves a grey unchanged) and include a control: a palette that is wrong must be reported as
wrong.

The colour maths were also compared once with the `colour-science` library, and agreed: luminance exactly, CIEDE2000
to within 0.002 on five test pairs, and the three simulation matrices identically. That comparison is not kept in the
repository.

## What is not covered

- **Status text on `surface` and `surface-2`.** The book draws status only on `bg`, so that is all the checker holds
  to 4.5:1. Some would fail elsewhere: light `status-success` is 4.23:1 on `surface` and 3.56:1 on `surface-2`, light
  `status-warning` is 4.39:1 and 3.69:1, and dark `status-important` is 4.41:1 on `surface-2`.
- **APCA** and the large-text thresholds. Only the WCAG 2 ratios are used.
- **The tritan simulation is an approximation.** `colour-science` documents Machado's tritan matrix as based on a
  shift model, not a model of true tritanopia.
- **Real users.** Simulation is not testing, and colour vision varies more than three matrices.
- **Hover, focus and selection.** The hover colours `--icons-hover` and `--theme-hover` are mapped, but no pair is
  checked for them. The text-selection colour is the browser's, and the copy button's hover filter is mdBook's own.
- **Surfaces the book does not map:** the search box and its results, the code-block highlight themes, and Mermaid
  diagrams. [How the book is themed](book-theme.md) lists them.
- **CI.** No job runs the checker. A change under `.config/` does start the whole Rust pipeline, because `.config/`
  is one of the paths it treats as Rust-relevant ([CI/CD](../architecture/cicd.md)), but that pipeline never looks at
  colours.
