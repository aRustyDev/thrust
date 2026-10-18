# Visual diff gate

Step 5 of the loop. Uses `scripts/pixeldiff.py` (path relative to this skill's directory, not the project's); what
it does, what it writes and its prerequisites are in `SKILL.md`, and `python3 scripts/pixeldiff.py ...` works in
place of `uv run --offline --script`.

The port is served by the user: `dx serve --web --port N --open false`. These flags parse on dioxus 0.7.10, and an
invalid value such as `--open maybe` is rejected with the possible values `true, false` `[verified: dioxus 0.7.10
(57d6794) argument parsing via --help; no server was started]`. If the project has a root `tailwind.css`, set
`NO_DOWNLOADS=1` first, or `dx` downloads a Tailwind binary.

## 1. What a green result does and does not prove

A pass means: at this viewport, DPR, browser and state, the two screenshots differ in no more than the
tolerance. It does not prove behaviour, keyboard/focus handling, accessibility, or any state you did not
capture. A green gate from a harness that has never failed proves nothing; section 3 is what makes it evidence.

## 2. Define the tolerance before the first comparison

`compare` takes `--tolerance-pct` (share of pixels allowed to differ, default 0.5) and `--channel-delta`
(how far any RGB channel may move before a pixel counts as different, default 8). **Those defaults are starting
points; calibrate them on your own pages:**

1. Capture the original twice with identical settings (reload between).
2. `compare` the two. The resulting `diff_pct` is the **noise floor** for that page.
3. Set the tolerance to at least twice the noise floor and no higher than you can justify. Write the number and
   the reason into `clone-report.md`. Any tolerance above 0.5 needs an explicit reason, and a tolerance above 5 or a
   `--channel-delta` above 32 also needs the user's approval: either can pass a wrong page.

Whole-page diffs hide small regressions inside a large page. For component-level ports, crop both sides with the
driver's element screenshot and compare crops.

## 3. The must-fail control (run it with every gate run)

The gate is only evidence if it demonstrably fails when the page is wrong. Run all three:

| Control | How | Expected |
|---|---|---|
| **Altered page** | In the port's page, before screenshotting, change something visible with the driver's evaluate function (append text to the main heading, or inject `body{background:#f0f}`), screenshot, `compare` against the original capture | exit 1, and a `hot_cell` or `bbox` over the altered element |
| **Wrong route** | compare the original's route A with the port's route B | exit 1 |
| **Tool self-test** | `uv run --offline --script scripts/pixeldiff.py selftest` | final line `SELFTEST PASS`, exit 0 |

If the altered-page control passes, it was too small (below tolerance) or the harness is broken (both screenshots
come from the same page, a stale file, a blank render). Make the alteration larger than the tolerance by
construction, then find the harness fault before trusting any green result. Never skip the control because the
last run was green.

The self-test has must-fail cases for a changed region, each single colour channel, a size mismatch, blank images
and a whole-image mask, plus pass cases for noise below the channel delta, a masked region, and palette and
gray+alpha PNGs decoding like RGB. It also checks that corrupt, truncated, 16-bit, interlaced, zero-size,
oversized (area above 50 million pixels, or a side above 20,000: the side cap stops a 1 x 50,000,000 image from
creating 50 million rows) and decompression-bomb files exit 2, that 20,000 on a side is still read, that `--out`
cannot overwrite an input (a hard link included), a symbolic link (a dangling one too, whose target would
otherwise be created) or a non-PNG file, and that each weakened setting prints an `OVERRIDE` line
`[verified: uv 0.12.18, python3 3.14.7]`.

Runtime, synthetic and unfiltered 1440x900 pairs on one machine (2026-10-18): about 0.5 s per `compare` with
unfiltered rows, about 1.7 s with all-Paeth RGBA rows, and about 1.3 to 2.5 s more with `--out`. Real screenshots
mix filter types and can take longer, so budget several seconds per pair `[unverified: no real screenshots timed]`.

## 4. Reading the output

The `RESULT` line is shown in `SKILL.md`. After it come up to five `hot_cell` lines (the busiest 64x64 cells,
largest first) and one `OVERRIDE` line per setting that departs from the defaults. Real output for a synthetic
1440x900 pair whose 400x60 altered region was **masked** and whose tolerance was raised to 2:

```text
RESULT PASS diff_pct=0.0000 diff_pixels=0 total=1296000 tolerance_pct=2.0 channel_delta=8 masked_pixels=24000 masked_pct=1.85 max_masked_pct=10.0 bbox=none
OVERRIDE tolerance_pct=2.0 (default 0.5): record the calibration and reason in clone-report.md
OVERRIDE masks=1: each needs a row with a reason in clone-report.md
```

A PASS that carries `OVERRIDE` lines is only as good as those reasons: every override goes in `clone-report.md`.
`--min-colors` below 8, `--max-masked-pct` above 10, `--channel-delta` above 32 and `--tolerance-pct` above 5 need
the user's approval first, and their `OVERRIDE` lines say so.

- `bbox` is `x0,y0,x1,y1` of every differing pixel (`none` when nothing differs).
- `--out diff.png` writes the original lightened with differing pixels in red. Look at it; fix the largest cell
  first; re-run. It must not name an input file.
- `size_mismatch` is never auto-resized. Fix viewport, DPR or scrollbar width first.
- `blank_image` means a capture is flat or an error page: re-capture.
- `masks_too_large` means the masks cover more than `--max-masked-pct` (default 10) of the image.
- Exit 2 means bad input: a file that is not an 8-bit, non-interlaced PNG (gray, RGB, palette, gray+alpha and RGBA
  are read), a corrupt or oversized file (above 50 million pixels, or above 20,000 on either side), the same file
  given twice (hard links and case variants included), or `--out` naming an input, a symbolic link or an existing
  file that is not a PNG.

Typical causes (heuristics, check each): whole-image offset suggests a scrollbar or body margin; one-pixel
shifts suggest `box-sizing`, line-height or border differences; colour-only deltas suggest a token or dark-mode
mismatch; text reflow suggests a font that did not load.

## 5. Masks are an exception register

`--mask X,Y,W,H` ignores a region. Each mask needs a row in `clone-report.md`:

| route | viewport | region | reason | revisit when |
|---|---|---|---|---|

Legitimate: an externally hosted avatar, a relative timestamp that cannot be frozen. Not legitimate: "does not
match yet". An unexplained mask is a failed gate. The tool enforces a ceiling as well: overlapping masks are
counted once, `masked_pct` is printed, and a run whose masks cover more than `--max-masked-pct` fails, so a mask
cannot swallow the page.

## 6. Limits

- Alpha is ignored; screenshots are expected to be opaque.
- Only 8-bit non-interlaced PNG is read. Re-save anything else as PNG, or use a driver that emits it.
- If the target renderer is the Blitz native renderer rather than a browser, a pixel comparison against a
  Chromium capture is not like for like; see `blitz-native` if installed, expect a higher tolerance, and justify it.
- Interactive states (hover, focus, open dialog) each need their own capture pair. Behavioural tests are out of
  scope for this skill.
