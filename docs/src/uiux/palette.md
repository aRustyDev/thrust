# Palette

**Draft.** The direction is set and the values are provisional: the design story is not finished. The tokens are in
`.config/branding/colors.css`, and this page reads them, so a swatch below always shows the current value. The checks
on them are on [Contrast and colour blindness](accessibility.md).

## Direction

The palette comes from iron pigments, because the whole spectrum exists in them. The statements about the pigments
below are the design direction's own, and have not been checked against a source here.

- **Ochre, russet and umber** make the brand and the neutral ramps.
- **Terre verte** (green earth) gets its colour from iron-bearing clays. It is the green.
- **Prussian blue** is an iron compound, and is generally considered the first modern synthetic pigment. It is the
  blue.
- **Mars black** is a synthetic iron oxide, ready-made for a true-dark theme. It is the dark base.

Two cautions come with it, and the rest of the design follows from them:

- Earth tones are low in chroma, so text contrast has to be checked, not assumed.
- Ochre, russet and umber can blur together for colour-blind users, so status must never rely on hue alone.

## Ramps

Each ramp runs from 50 (lightest) to 900 (darkest). OKLCH lightness goes from 0.975 to 0.26, in smaller steps at the
light end (0.03 up to 0.10), with chroma kept low and pulled back into the sRGB gamut. The ramp values were generated
once from those targets by a drafting helper that is not in the repository. The CSS file is the source, and nothing
regenerates it. Which ramp step each role takes was chosen by hand, and the checker holds it to the rules below.

**Ochre**, brand (yellow iron oxide)<br>
<span class="swatch" title="ochre-50" style="--c: var(--ochre-50)"></span><span class="swatch" title="ochre-100" style="--c: var(--ochre-100)"></span><span class="swatch" title="ochre-200" style="--c: var(--ochre-200)"></span><span class="swatch" title="ochre-300" style="--c: var(--ochre-300)"></span><span class="swatch" title="ochre-400" style="--c: var(--ochre-400)"></span><span class="swatch" title="ochre-500" style="--c: var(--ochre-500)"></span><span class="swatch" title="ochre-600" style="--c: var(--ochre-600)"></span><span class="swatch" title="ochre-700" style="--c: var(--ochre-700)"></span><span class="swatch" title="ochre-800" style="--c: var(--ochre-800)"></span><span class="swatch" title="ochre-900" style="--c: var(--ochre-900)"></span>

**Russet**, brand (a red-brown; the pigments behind it are the red iron oxides)<br>
<span class="swatch" title="russet-50" style="--c: var(--russet-50)"></span><span class="swatch" title="russet-100" style="--c: var(--russet-100)"></span><span class="swatch" title="russet-200" style="--c: var(--russet-200)"></span><span class="swatch" title="russet-300" style="--c: var(--russet-300)"></span><span class="swatch" title="russet-400" style="--c: var(--russet-400)"></span><span class="swatch" title="russet-500" style="--c: var(--russet-500)"></span><span class="swatch" title="russet-600" style="--c: var(--russet-600)"></span><span class="swatch" title="russet-700" style="--c: var(--russet-700)"></span><span class="swatch" title="russet-800" style="--c: var(--russet-800)"></span><span class="swatch" title="russet-900" style="--c: var(--russet-900)"></span>

**Umber**, the warm neutral ramp<br>
<span class="swatch" title="umber-50" style="--c: var(--umber-50)"></span><span class="swatch" title="umber-100" style="--c: var(--umber-100)"></span><span class="swatch" title="umber-200" style="--c: var(--umber-200)"></span><span class="swatch" title="umber-300" style="--c: var(--umber-300)"></span><span class="swatch" title="umber-400" style="--c: var(--umber-400)"></span><span class="swatch" title="umber-500" style="--c: var(--umber-500)"></span><span class="swatch" title="umber-600" style="--c: var(--umber-600)"></span><span class="swatch" title="umber-700" style="--c: var(--umber-700)"></span><span class="swatch" title="umber-800" style="--c: var(--umber-800)"></span><span class="swatch" title="umber-900" style="--c: var(--umber-900)"></span>

**Terre verte**, success<br>
<span class="swatch" title="terre-verte-50" style="--c: var(--terre-verte-50)"></span><span class="swatch" title="terre-verte-100" style="--c: var(--terre-verte-100)"></span><span class="swatch" title="terre-verte-200" style="--c: var(--terre-verte-200)"></span><span class="swatch" title="terre-verte-300" style="--c: var(--terre-verte-300)"></span><span class="swatch" title="terre-verte-400" style="--c: var(--terre-verte-400)"></span><span class="swatch" title="terre-verte-500" style="--c: var(--terre-verte-500)"></span><span class="swatch" title="terre-verte-600" style="--c: var(--terre-verte-600)"></span><span class="swatch" title="terre-verte-700" style="--c: var(--terre-verte-700)"></span><span class="swatch" title="terre-verte-800" style="--c: var(--terre-verte-800)"></span><span class="swatch" title="terre-verte-900" style="--c: var(--terre-verte-900)"></span>

**Prussian blue**, information and links<br>
<span class="swatch" title="prussian-50" style="--c: var(--prussian-50)"></span><span class="swatch" title="prussian-100" style="--c: var(--prussian-100)"></span><span class="swatch" title="prussian-200" style="--c: var(--prussian-200)"></span><span class="swatch" title="prussian-300" style="--c: var(--prussian-300)"></span><span class="swatch" title="prussian-400" style="--c: var(--prussian-400)"></span><span class="swatch" title="prussian-500" style="--c: var(--prussian-500)"></span><span class="swatch" title="prussian-600" style="--c: var(--prussian-600)"></span><span class="swatch" title="prussian-700" style="--c: var(--prussian-700)"></span><span class="swatch" title="prussian-800" style="--c: var(--prussian-800)"></span><span class="swatch" title="prussian-900" style="--c: var(--prussian-900)"></span>

**Mars black**, the true-dark base and three lifts<br>
<span class="swatch" title="mars-black" style="--c: var(--mars-black)"></span><span class="swatch" title="mars-900" style="--c: var(--mars-900)"></span><span class="swatch" title="mars-800" style="--c: var(--mars-800)"></span><span class="swatch" title="mars-700" style="--c: var(--mars-700)"></span>

## Roles

A role has one value per theme. Which ramp step each role takes is in `.config/branding/colors.css`, so it is not
repeated here.

| Role | Light | Dark | Used for |
|---|---|---|---|
| `bg` | <span class="swatch" style="--c: var(--light-bg)"></span> | <span class="swatch" style="--c: var(--dark-bg)"></span> | The page |
| `surface` | <span class="swatch" style="--c: var(--light-surface)"></span> | <span class="swatch" style="--c: var(--dark-surface)"></span> | The sidebar and panels |
| `surface-2` | <span class="swatch" style="--c: var(--light-surface-2)"></span> | <span class="swatch" style="--c: var(--dark-surface-2)"></span> | Quotes and table headers |
| `border` | <span class="swatch" style="--c: var(--light-border)"></span> | <span class="swatch" style="--c: var(--dark-border)"></span> | Rules and borders |
| `fg` | <span class="swatch" style="--c: var(--light-fg)"></span> | <span class="swatch" style="--c: var(--dark-fg)"></span> | Body text |
| `fg-muted` | <span class="swatch" style="--c: var(--light-fg-muted)"></span> | <span class="swatch" style="--c: var(--dark-fg-muted)"></span> | Secondary text and icons |
| `link` | <span class="swatch" style="--c: var(--light-link)"></span> | <span class="swatch" style="--c: var(--dark-link)"></span> | Links |
| `accent` | <span class="swatch" style="--c: var(--light-accent)"></span> | <span class="swatch" style="--c: var(--dark-accent)"></span> | A fill or marker, never text |
| `on-accent` | <span class="swatch" style="--c: var(--light-on-accent)"></span> | <span class="swatch" style="--c: var(--dark-on-accent)"></span> | Text on an accent fill |
| `accent-text` | <span class="swatch" style="--c: var(--light-accent-text)"></span> | <span class="swatch" style="--c: var(--dark-accent-text)"></span> | The accent where it has to be read: the active item, inline code |
| `mark` | <span class="swatch" style="--c: var(--light-mark)"></span> | <span class="swatch" style="--c: var(--dark-mark)"></span> | Highlights: a search match in the page, and the flash behind a footnote reference |
| `status-info` | <span class="swatch" style="--c: var(--light-status-info)"></span> | <span class="swatch" style="--c: var(--dark-status-info)"></span> | Notes |
| `status-success` | <span class="swatch" style="--c: var(--light-status-success)"></span> | <span class="swatch" style="--c: var(--dark-status-success)"></span> | Tips |
| `status-warning` | <span class="swatch" style="--c: var(--light-status-warning)"></span> | <span class="swatch" style="--c: var(--dark-status-warning)"></span> | Warnings |
| `status-danger` | <span class="swatch" style="--c: var(--light-status-danger)"></span> | <span class="swatch" style="--c: var(--dark-status-danger)"></span> | Cautions |
| `status-important` | <span class="swatch" style="--c: var(--light-status-important)"></span> | <span class="swatch" style="--c: var(--dark-status-important)"></span> | Important notes, a neutral brown |

The light theme is warm paper and dark umber text. The dark theme is Mars black with umber text, and the accents move
to the lighter ochre steps so they stay readable.

## Open design questions

The story is not complete. These are undecided, and the draft makes a choice for each only so that there is
something to look at.

- **The primary brand hue.** The draft uses ochre for fills and markers and russet for accent text on the light
  theme. It could be the other way round, or a single hue.
- **A pigment per theme.** mdBook has five themes, and the draft gives them two looks. Each could have its own
  pigment instead: Mars black for the true dark, a Prussian-blue dark, an umber dark, a russet light.
- **Prussian blue as a surface.** Today it appears only as the link and information colour. It is not a background.
- **The fifth status.** mdBook has five alert kinds and the palette has four status hues, so `important` is a neutral
  brown, which leans on its label and icon more than the others do.
- **Everything that is not colour.** Typography, logo, favicon and the product interface are not designed yet.
- **Diagrams.** Mermaid diagrams keep mermaid's own colours. See [How the book is themed](book-theme.md).
