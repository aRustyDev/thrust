# Theme and CSS variables (dioxus-components registry, Dioxus 0.7)

Sources: `preview/assets/dx-components-theme.css`, `preview/src/components/button/{component.rs,style.css}`
and `component.json` in `DioxusLabs/dioxus-components` `main` (2026-10-18); the Dioxus 0.7 Styling page.
The registry's `main` moves: re-read the theme file at the SHA that is pinned. Tailwind is documented in
the `dioxus-07` skill (`references/cheatsheet.md`, section 8); this file does not repeat it.

## What gets installed

`dx components add` copies `dx-components-theme.css` into the assets directory and the component folders
into `src/components/`. Link the theme **once** at the app root (it is global):

```rust
rsx! {
    document::Stylesheet { href: asset!("/assets/dx-components-theme.css") }
    Router::<Route> {}
}
```

Each component folder has its own `style.css`, loaded through a CSS module (an attribute macro):

```rust
#[css_module("/src/components/button/style.css")]
struct Styles;                       // Styles::dx_button -> the (hashed) class for .dx-button
```

`#[css_module(..)]` is exported by `manganis` and re-exported through the Dioxus prelude (0.7.10).
Component styling hooks are **attributes**, not class names: `data-style="primary|secondary|destructive|outline|ghost|link"`
and `data-size="xs|sm|default|lg|icon|icon-xs|icon-sm|icon-lg"` on Button (variants are Rust enums).
Primitives expose `data-state`, `data-disabled`, `data-orientation`, `data-side`, `data-align` for CSS.

## Variables in the theme file

| Group | Names |
|---|---|
| Surfaces (light and dark pair) | `--primary-color`, `--primary-color-1` .. `--primary-color-7` |
| Text and contrast | `--secondary-color`, `--secondary-color-1` .. `--secondary-color-6` |
| Focus | `--focused-border-color` (default `#2b7fff` in both schemes) |
| Status | `--primary-success-color`, `--secondary-success-color`, `--primary-warning-color`, `--secondary-warning-color`, `--primary-error-color`, `--secondary-error-color`, `--contrast-error-color`, `--primary-info-color`, `--secondary-info-color` |
| Scheme switches | `--dark`, `--light`, `--dxc-dark-on`, `--dxc-light-on` |

Each colour is declared as `var(--dark, <dark value>) var(--light, <light value>)`: exactly one of
`--dark` / `--light` is `initial` and the other is empty, which makes the unused branch vanish. When
re-declaring one of these variables, re-declare both the light and the dark branch.

## Dark and light mode

- Default follows `@media (prefers-color-scheme: ...)`.
- Force a scheme with `<html data-theme="dark">` or `data-theme="light"` (it wins over the media query by
  selector specificity). A theme toggle sets that attribute (for example with `document::eval(..)` or an
  event handler); persist the choice in the app.
- `[class~="dxc-system"]` scopes the scheme switch for the gallery's code samples; app code does not need
  it. `[unverified: intended use beyond the gallery app]`.

## Overriding without forking

1. Load the override stylesheet **after** `dx-components-theme.css`.
2. Re-declare variables on `:root` using the same two-branch form, for example
   `:root { --focused-border-color: var(--dark, #7aa2ff) var(--light, #1d4ed8); }`.
3. For one component, use a wrapper selector on the stable **data attributes**, not the class name (the
   module hashes class names), and include the element name so the selector outranks the component's own
   `.dx-button[data-style="primary"]` rule (specificity (0,2,1) against (0,2,0)):
   `.settings-panel button[data-style="primary"] { background-color: var(--secondary-color-4); }`
   Hover, focus and disabled states have their own, more specific rules in `style.css` (the primary hover
   rule `.dx-button[data-style="primary"]:not(:disabled):hover` is (0,4,0)), so each state you restyle
   needs its own override at equal or higher specificity.
4. Edit the copied `style.css` only when the project accepts owning that file; `add --force` overwrites it.

## Gotchas

- Unstyled output almost always means the theme stylesheet is not linked or `mod components;` is missing.
- Components assume the Dioxus 0.7.8 baseline declared in the registry workspace (`workflow.md`).
- The registry README mentions `/assets/dx-components.css`; the CLI and manifests use
  `dx-components-theme.css`.
- The registry components are plain CSS modules and need no Tailwind. If both are used, a sensible order
  is the theme file, then the Tailwind output, then project overrides `[unverified: Tailwind v4 layers can
  change which rule wins; check the cascade in the browser]`. Tailwind setup is in `dioxus-07`.
