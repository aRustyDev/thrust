# shadcn/ui and Radix names to dioxus-components

Basis: the registry index at `DioxusLabs/dioxus-components` `main` on 2026-10-18 (45 styled components under
`preview/src/components/` at that date) and the module list in the `dioxus-primitives` crate's
`primitives/src/lib.rs`. Names are snake_case and are what `dx components add <name>` takes. Confirm
against the registry index at the SHA in use (`dx components list`, which the user runs): this list drifts.

**Match column:**
- `same-name`: the name is the same in both ecosystems. The job is assumed to be the same; behaviour was
  not compared or run.
- `nearest`: the closest equivalent; behaviour differs and is `[unverified]`.
- `none`: no registry entry. Build it (see the `dioxus-07` skill) and never invent a name for
  `dx components add`.

| shadcn / Radix | dioxus-components | Unstyled primitive module | Match |
|---|---|---|---|
| Accordion | `accordion` | `accordion` | same-name |
| Alert Dialog | `alert_dialog` | `alert_dialog` | same-name |
| Aspect Ratio | `aspect_ratio` | `aspect_ratio` | same-name |
| Avatar | `avatar` | `avatar` | same-name |
| Badge | `badge` | none listed | same-name (styled only) |
| Button | `button` | none listed | same-name (styled only; `variant`, `size` enums) |
| Calendar | `calendar` | `calendar` | same-name |
| Card | `card` | none listed | same-name (styled only) |
| Checkbox | `checkbox` | `checkbox` | same-name |
| Collapsible | `collapsible` | `collapsible` | same-name |
| Combobox | `combobox` | `combobox` | same-name |
| Context Menu | `context_menu` | `context_menu` | same-name |
| Date Picker | `date_picker` | `date_picker` | same-name (also copies `calendar`, `popover`) |
| Dialog | `dialog` | `dialog` | same-name |
| Dropdown Menu | `dropdown_menu` | `dropdown_menu` | same-name |
| Form | `form` | none listed | nearest (not react-hook-form) |
| Hover Card | `hover_card` | `hover_card` | same-name |
| Input | `input` | none listed | same-name (styled only) |
| Item | `item` | none listed | same-name (styled only; also copies `separator`) |
| Label | `label` | `label` | same-name |
| Menubar | `menubar` | `menubar` | same-name |
| Navigation Menu (Radix) | `navbar` | `navbar` | nearest |
| Pagination | `pagination` | none listed | same-name (styled only) |
| Popover | `popover` | `popover` | same-name |
| Progress | `progress` | `progress` | same-name |
| Radio Group | `radio_group` | `radio_group` | same-name |
| Scroll Area | `scroll_area` | `scroll_area` | same-name |
| Select | `select` | `select` | same-name |
| Separator | `separator` | `separator` | same-name |
| Sheet | `sheet` | none listed | same-name (styled only) |
| Sidebar | `sidebar` | none listed | same-name (styled only; also copies `button`, `separator`, `sheet`, `skeleton`, `tooltip`) |
| Skeleton | `skeleton` | none listed | same-name (styled only) |
| Slider | `slider` | `slider` | same-name |
| Sonner / Toast | `toast` | `toast` | nearest (Sonner is a separate library) |
| Switch | `switch` | `switch` | same-name |
| Tabs | `tabs` | `tabs` | same-name |
| Textarea | `textarea` | none listed | same-name (styled only) |
| Toggle | `toggle` | `toggle` | same-name |
| Toggle Group | `toggle_group` | `toggle_group` | same-name |
| Toolbar (Radix) | `toolbar` | `toolbar` | same-name |
| Tooltip | `tooltip` | `tooltip` | same-name |

Components that declare other registry components (`date_picker`, `color_picker`, `item`, `sidebar`) are
affected by the pinned-add abort described in `workflow.md` (DioxusLabs/dioxus#5310): add their dependency
components first, each in its own command.

"None listed" means no module of that name appears in `primitives/src/lib.rs`; the file also contains
`cfg`-gated lines that were not expanded, and a styled-only component may still compose other primitives
`[unverified: composition of styled-only components]`.

**Registry-only (no shadcn counterpart by name):** `color_picker`, `drag_and_drop_list`, `tag_group`,
`virtual_list`.

**`none` (no registry entry as of the read date):** Table, Breadcrumb, Carousel, Chart, Command, Drawer
(vaul), Input OTP, Resizable, Typography, Spinner, Kbd, Field, Button Group, Native Select, Empty. For a
drawer, `sheet` is the nearest side-panel component, but Drawer itself is `none`. `dx components add
<name>` for any of these fails with `Component '<name>' not found in registry`.

## Idiom translation (read from the primitives source)

| Radix / React idiom | Dioxus primitives |
|---|---|
| controlled `open` / `onOpenChange` / `defaultOpen` | props `open: ReadSignal<Option<bool>>`, `on_open_change: Callback<bool>`, `default_open: bool` (Dialog). Same trio pattern: `value`, `on_value_change`, `default_value` (Tabs) |
| controlled vs uncontrolled state | `use_controlled(prop, default, on_change)` in `dioxus-primitives` |
| `asChild` | no equivalent; pass extra attributes through the `attributes: Vec<Attribute>` prop (`#[props(extends = GlobalAttributes)]`) and merge with `merge_attributes(vec![base, attributes])` (later lists win; `class` values are joined) |
| `data-state="open"` styling | same attribute names (`data-state`, `data-disabled`, `data-orientation`, `data-side`, `data-align`) |
| generated ids for `aria-labelledby` | automatic (`dxc-<n>` ids, server-stable under fullstack); pass `id` to override |
| `modal` | Dialog prop `is_modal` (default `true`: traps focus) |
| roving focus | Tabs props `horizontal`, `roving_loop` |

A compound component is a set of exported parts (for example `DialogRoot` plus content, title and
description parts) that you compose. Read the copied `component.rs` of the component you added for the exact
part names rather than guessing from Radix.
