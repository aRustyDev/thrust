# How the book is themed

The book takes its colours from `.config/branding/colors.css`, the same tokens the [palette](palette.md) describes. This
page is the wiring: which file does what, what the mapping covers, and what it leaves alone. The palette is a draft,
and so is this.

## The files

| File | Does |
|---|---|
| `.config/branding/colors.css` | The brand tokens: ramps, and a light and a dark value for each role. It names no consumer's variables |
| `docs/branding` | A symlink to `../.config/branding`, so the book can load the tokens without copying them |
| `docs/brand-mapping.css` | Maps the tokens onto mdBook's theme variables, and defines the palette page's swatches |
| `docs/book.toml` | `additional-css = ["branding/colors.css", "brand-mapping.css"]`. mdBook loads these after its own stylesheets, which is what lets the mapping override its variables. The tokens are listed first for readability, and a `var()` resolves whichever file comes first |
| `.config/branding/check.py` | The contrast and colour-blindness checker ([Contrast and colour blindness](accessibility.md)) |

## Why a symlink

Pointing `additional-css` straight at `../.config/branding/...` does not work. mdBook accepts the path, and the build
succeeds, but it copies the file to `docs/.config/branding/`, a stray directory outside the output folder, and the
generated pages link to `../.config/branding/...`, which a published site does not have `[verified 2026-10-19, mdBook
0.5.4]`. Through the symlink the file is copied into the output with a hashed name, and the links resolve from the
root and from sub-pages `[verified 2026-10-19, mdBook 0.5.4]`.

## What the mapping covers

mdBook has five themes. The two light ones (`light` and `rust`) take the light tokens, and the three dark ones (`coal`,
`navy` and `ayu`) take the dark tokens, so the theme picker shows five names for two looks.

With JavaScript off, the page has no `js` class on its `html` element, and mdBook's own `html:not(.js)` rule makes it
follow the operating system. The mapping repeats that rule with the brand tokens. This is `[derived]` from mdBook's
stylesheet and was not run with JavaScript off.

Each of the five themes was built and a sample of its computed variables read in a browser (the page, text and
sidebar backgrounds, links, two alert colours and the search highlight), and they match the tokens
`[verified 2026-10-19, draft palette]`.

Mapped: the page, text, sidebar, links, inline code colour, the theme popup, quotes, tables, scrollbar and icons, the
warning border, the two highlights (a search match in the page and the footnote flash, which both take `mark`), and the
five alert colours (note is `status-info`, tip is `status-success`, important is `status-important`, warning is `status-warning`,
caution is `status-danger`).

Not mapped, so they stay mdBook's own:

- the search box and its results, and the copy-button icon (it is coloured with a CSS filter chain, not a colour);
- the overlay that dims the book behind the keyboard-help popup, and `--color-scheme`, which each mdBook theme keeps;
- the code-block highlight themes, so a code block, and the background behind inline code, keep mdBook's own colours.
  The inline code colour is mapped, and the checker holds it to 4.5:1 on each of those backgrounds;
- **Mermaid diagrams.** `mermaid-init.js` picks mermaid's own `default` theme for the light themes and `dark` for the
  dark ones, so the diagrams keep mermaid's colours. Branding them would mean replacing that generated file with one
  that passes mermaid theme variables, which could read them from the same tokens. That is not done or tested.

## Changing a colour

1. Edit the token in `.config/branding/colors.css`.
2. Run `uv run .config/branding/check.py`. A failing contrast pair is exit 1.
3. Build with `mdbook build docs` and look at the light and dark themes. The checker cannot say whether it looks right.

In a page, show a swatch with `<span class="swatch" style="--c: var(--ochre-500)"></span>`, and never a hex value, so the
page cannot drift from the file.

## Pitfalls

- **A misspelled token fails silently.** The mapping then points at nothing, the variable resolves to empty, and the
  browser falls back to its default: links lose their colour and the build says nothing `[verified 2026-10-19, with the
  tokens file emptied]`.
- **A missing file does fail the build.** A dangling `docs/branding` link, or a missing `colors.css`, stops
  `mdbook build` with exit 101 `[verified 2026-10-19, with a dangling link]`. No CI job builds the book, so this shows
  up only on a machine that builds it ([CI/CD](../architecture/cicd.md)).
- **The symlink needs a platform that supports them.** The repository already relies on symlinks (`.gitnexusrc` and
  `.claude/skills` are two).
- **A palette change runs the whole Rust pipeline.** `.config/` is one of the paths the CI classifier treats as
  Rust-relevant, because `.config/nextest.toml` lives there, so a change under `.config/branding/` starts every job.
  Narrowing that path in `.github/scripts/classify-changes.sh` is a separate change.
