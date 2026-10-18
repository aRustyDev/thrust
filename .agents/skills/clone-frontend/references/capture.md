# Capture and tokens

Steps 1 and 2 of the loop. The in-page JS snippets are tagged `[unverified: no browser driver]`: test each on one
page before trusting a bundle built with them. The shell checks in sections 5 and 6 are verified, each with a
control that must fail. Clone only UIs you have the right to reproduce.

## 1. Driver contract (tool-agnostic)

Any browser-automation MCP server or CLI works if it can do all of:

| Capability | Why |
|---|---|
| navigate, set viewport and device scale factor | the diff only means something at identical size |
| evaluate JS in the page and return JSON (or write it to a file) | DOM, computed styles, stylesheet rules |
| save a PNG screenshot to a file | input to `scripts/pixeldiff.py` |
| freeze animation (reduced-motion emulation, or inject CSS) | animated frames never diff clean |
| wait for fonts and network idle | unloaded web fonts shift every line |

Playwright- and Chrome-DevTools-based MCP servers advertise these; check the tool list of whatever you have
rather than assuming. No tool or version is claimed here. If the driver cannot return PNG bytes to disk, the
diff step cannot run: stop and say so.

Freeze-CSS to inject before every screenshot, original and port alike:

```css
*,*::before,*::after{animation:none!important;transition:none!important;caret-color:transparent!important}
```

## 2. Where to capture from, and determinism

**Source of the capture.** A seeded development instance of the original, opened in a **fresh browser profile**
signed in as a **throwaway account**; or a public page that needs no sign-in, opened in a fresh profile. Never
capture a signed-in production session, and never use the user's own browser profile or account: it carries their
sessions, cookies and personal data.

Determinism settings (record all in `manifest.json`):

- Viewports: a fixed list (for example 1440x900, 768x1024, 390x844). Device scale factor 1 (smaller PNGs; the
  bundled differ is pure Python).
- One colour scheme per capture; light and dark are separate captures.
- Same browser engine and version for original and port. Record it.
- Seeded data only. Time-dependent text (relative dates, clocks) is stubbed or masked with a written reason.
- Locale and timezone fixed.

## 3. Bundle layout

```text
capture/
  manifest.json            routes, viewports, dpr, scheme, browser + driver versions, seed note, date
  <route>/<viewport>/
    shot.png
    dom.html               sanitised app-root markup
    frag-<name>.html       one fragment cut from dom.html, the input to `dx translate` (several per route, one per file)
    frag-<name>.rsx        the raw `dx translate` output for that fragment (`translate.md`)
    styles.json            computed styles keyed by selector path
    states/<name>.png      open menus, hover, focus: drive the original into the state first
```

Exit check: `manifest.json` lists every route x viewport you intend to port, every `shot.png` exists, and none is
blank (the differ's blank guard will also fail a flat image); add `"scan_reviewed": []`. Then run the scan in section 6.

## 4. What to capture, and the limits of the DOM

**DOM (sanitised).** Take the app root, not `document`. Remove `<script>`, `<noscript>`, `<style>`, stylesheet
links, comment nodes (they can carry anything), `value` attributes on inputs, attributes whose names contain a
secret-like word as a whole segment (`data-csrf-token`, `data-key`, `data-password`, `data-signature`; not `aria-*`
and not `data-author`) or a concatenated form (`apikey`, `sessionid`, `accesstoken`, `clientsecret`, ...), and CSRF
`<meta>` tags; strip query strings from every URL attribute including `xlink:href` (they often carry signatures
and tokens). The `#fragment` is kept, and OAuth implicit-flow tokens live there, so the scan in section 6 is the
only catch for those. Never save cookies, `localStorage`, or request headers.

```js
// [unverified: no browser driver]
(() => {
  const noQ = u => u.replace(/\?[^#"')\s]*/g, '');                 // drop ?query, keep #fragment
  const secretName = n => !/^aria-/i.test(n) && (
    /(^|[-_:])(token|secret|csrf|xsrf|auth|authorization|session|key|nonce|password|passwd|pwd|bearer|jwt|signature|sig|cookie|credentials?)([-_:]|$)/i.test(n)
    || /apikey|sessionid|(csrf|xsrf|access|id|refresh|session)token|secretkey|clientsecret|privatekey/i.test(n));
  const root = document.querySelector('#root, #app, main');
  const c = root.cloneNode(true);
  c.querySelectorAll('script,noscript,style,link[rel=stylesheet],meta[name*=csrf i]').forEach(n => n.remove());
  const tw = document.createTreeWalker(c, NodeFilter.SHOW_COMMENT);
  const comments = []; while (tw.nextNode()) comments.push(tw.currentNode);
  comments.forEach(n => n.remove());
  c.querySelectorAll('input,textarea').forEach(n => n.removeAttribute('value'));
  c.querySelectorAll('*').forEach(n => {
    for (const a of [...n.attributes]) {
      if (secretName(a.name)) n.removeAttribute(a.name);
      else if (['href', 'xlink:href', 'src', 'srcset', 'poster', 'action'].includes(a.name)) n.setAttribute(a.name, noQ(a.value));
      else if (a.name === 'style') n.setAttribute('style', a.value.replace(/url\(([^)]*)\)/g, (_m, u) => 'url(' + noQ(u) + ')'));
    }
  });
  return c.outerHTML;
})()
```

The string helpers in that snippet (`noQ`, the `url(...)` rewrite, `secretName`) were run on sample strings and behave
as described `[verified: node v26.9.0]`: 34 secret-like names are removed (including `data-accesstoken`,
`data-clientsecret`, `data-privatekey`), and 24 others are kept (including `aria-keyshortcuts`, `aria-label`,
`data-author`, `data-signal`, `data-design`, `data-tokenizer`, `data-secretary`); the DOM wiring (comment removal, attribute loop) is
`[unverified: no browser driver]`.

**The DOM cannot see framework event handlers.** A React app attaches handlers in JS, not as `on*` attributes, so
`dom.html` shows structure and never behaviour. Read behaviour from the original's source (if that source is
outside the project, ask the user to name its path first and read only that path); for non-trivial logic use the
`ts-to-rust` method (if installed). State-dependent markup (dialogs, dropdowns, tabs) only exists after you drive the
original into that state, so script those clicks and capture each state separately.

**Computed styles.** Record longhand properties only (shorthands are inconsistent across engines), keyed by a
stable selector path. Values containing `url(...)` get their query strings stripped:

```js
// [unverified: no browser driver]
(() => {
  const noQ = u => u.replace(/\?[^#"')\s]*/g, '');
  const S = ['top','right','bottom','left'];
  const P = ['display','position','width','height','color','background-color','background-image',
    'font-family','font-size','font-weight','line-height','letter-spacing','text-align','text-transform',
    'border-top-left-radius','box-shadow','opacity','gap','flex-direction','justify-content','align-items',
    'grid-template-columns','overflow-x','overflow-y','z-index','cursor',
    ...S.flatMap(s => ['margin-'+s,'padding-'+s,'border-'+s+'-width','border-'+s+'-color'])];
  const sel = el => { const a = []; for (let n = el; n && n.nodeType === 1 && n.parentElement; n = n.parentElement) {
    const i = [...n.parentElement.children].filter(x => x.tagName === n.tagName).indexOf(n) + 1;
    a.unshift(n.tagName.toLowerCase() + ':nth-of-type(' + i + ')'); } return a.join(' > '); };
  const val = v => v.replace(/url\(([^)]*)\)/g, (_m, u) => 'url(' + noQ(u) + ')');
  const out = {};
  document.querySelectorAll('#root *, #app *, main *').forEach(el => {
    const cs = getComputedStyle(el); out[sel(el)] = Object.fromEntries(P.map(p => [p, val(cs.getPropertyValue(p))]));
  });
  return JSON.stringify(out);
})()
```

## 5. Tokens

Goal: `tokens.css` that makes every colour, size and radius the port uses a named variable.

1. **Prefer the source.** Read the original's stylesheet and utility-CSS config directly; if they are outside the
   project, ask the user to name the path first and read only that path. Runtime scraping is the cross-check, not
   the primary source.
2. **Scrape custom properties** from the running original, skipping cross-origin sheets:

```js
// [unverified: no browser driver]
(() => {
  const out = {};
  for (const sheet of document.styleSheets) {
    let rules; try { rules = sheet.cssRules; } catch { continue; }
    for (const r of rules) {
      if (r.style && /^(:root|\.dark|\[data-theme[^\]]*\])$/.test(r.selectorText || '')) {
        for (const name of r.style) if (name.startsWith('--'))
          (out[r.selectorText] ??= {})[name] = r.style.getPropertyValue(name).trim();
      }
    }
  }
  return JSON.stringify(out);
})()
```

3. **Derive scales** from `styles.json`: count font sizes, line heights, spacings, radii and colours. Values used
   repeatedly become tokens. Merge only exact duplicates; list near-duplicates in the report instead of merging
   silently.
4. **Write `tokens.css`** with `:root {}` and a dark block; reuse the original's variable names where they exist.
   Theme-variable conventions for the component library belong to `dioxus-design-system`, not here.
5. **Fonts and images:** record family names and licences. Do not copy proprietary font or image files into the
   port.

**Exit check: every variable used is defined.** Scope: `var(--x)` references in `src/` and in `tokens.css` itself
are checked against the definitions in `tokens.css`. Add every other stylesheet the project loads (for example
`assets/*.css` or a root `tailwind.css`) to `used`, and to `defined` when it also defines variables; one that is not
listed is not checked. Names may contain underscores and capitals, and a definition may sit on one line with
others. The command uses process substitution and arrays, so it needs bash or zsh (not plain `sh`), and it writes
no files. A missing `src/` or `tokens.css` is reported as not run, never as "all defined":

```sh
used=(src tokens.css)     # add other stylesheets the project loads
defined=(tokens.css)      # add those that also define variables
if [ -d src ] && [ -s tokens.css ]; then
  missing=$(comm -23 <(grep -rhoE 'var\(--[A-Za-z0-9_-]+' "${used[@]}" | sed 's/^var(//' | sort -u) \
                     <(grep -hoE '(^|[{;[:space:]])--[A-Za-z0-9_-]+[[:space:]]*:' "${defined[@]}" | sed -E 's/^[{;[:space:]]*//; s/[[:space:]]*:$//' | sort -u))
  if [ -z "$missing" ]; then echo ALL DEFINED; else printf 'UNDEFINED:\n%s\n' "$missing"; fi
else echo "TOKEN CHECK NOT RUN: src/ or tokens.css missing"; fi
```

Controls, all as expected: a `var(--nope_x)` added under `src` is listed; a `var(--zzz)` inside `tokens.css` is
listed; a stylesheet outside the lists is not checked until added; a missing `src/` and a missing `tokens.css` each
print `TOKEN CHECK NOT RUN`. Tested with `--my_var : 3`, `--Radius-md:4px` and a one-line
`:root { --a: 1; --b-c: 2 }`, with identical output under bash 5.3.15 and zsh 5.9.2 and the working directory
unchanged afterwards `[verified: grep (BSD grep, GNU compatible) 2.6.0-FreeBSD, sed, comm]`.

## 6. Before checking in `capture/`: scan and review

The sanitising snippets reduce, not eliminate, sensitive content. A checked-in `capture/` in a public repository
can expose personal data or proprietary content that appeared on the page.

```sh
grep -rIliE 'authorization|cookie|bearer|api[_-]?key|secret|token|passw(or)?d|pwd|credential|jwt|[xc]srf|session[_-]?id|signature|[?&](key|sig|session)=|eyJ[A-Za-z0-9_-]{10,}|BEGIN [A-Z ]*PRIVATE KEY' capture
```

The scan lists **file names only**, so a matched secret is not echoed. `-I` skips binary files: without it, the
compressed bytes of a PNG can contain a short word such as `jwt` by chance and be listed (a 300x200 noise PNG did).
Screenshots are therefore not scanned; open them and look. A clean capture prints nothing and exits 1.

Controls: plant one file per name, for example `<div data-password="x">`, `<div data-apikey="x">`,
`<div data-signature="x">`, `<a href="/x?token=abc123">` and `<div style="background-image:url(/a.png?sig=abc123)">`,
and confirm each is listed and a clean file is not (23 planted files were all listed, under bash 5.3.15 and zsh
5.9.2); a PNG containing `JwT` is listed without `-I` and not with it
`[verified: grep (BSD grep, GNU compatible) 2.6.0-FreeBSD]`. The pattern matches ordinary words, so expect hits on
plain UI text, and `manifest.json` itself once its `scan_reviewed` entries quote a word such as "Password": add
`--exclude=manifest.json` (it holds no page content) or expect it in the list. For each listed file, open it and
decide:

- a hit inside an attribute, URL, script or data blob: remove it from the capture and re-scan;
- a hit that is plain visible UI text (a `<p>Password</p>` label): record it as reviewed in `manifest.json`
  (`"scan_reviewed": ["<path>: <reason>"]`) and leave it.

Then review `capture/` by hand and ask the user whether screenshots and DOM should be checked in or git-ignored;
do not edit `.gitignore` yourself.
