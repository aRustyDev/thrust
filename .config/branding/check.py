#!/usr/bin/env python3
"""Check the brand tokens in colors.css. Standard library only.

  uv run .config/branding/check.py [--css PATH]

1. Contrast (a gate): every text pair in PAIRS must reach its WCAG 2 contrast ratio, per theme. The run exits 1
   if one does not, or if a token cannot be resolved. Exit 2 means a self-test failed, and 3 that the token file
   could not be read.
2. Colour blindness (a report): how far apart the status colours stay under protanopia, deuteranopia and
   tritanopia (Machado 2009, severity 1.0), as CIEDE2000. A pair below DISTINCT is not told apart by colour
   alone, so it must keep a second cue (an icon, a label).

Known-answer self-tests run first, and the run stops if one fails. A check that cannot fail is not evidence.
"""
import math
import re
import sys
from pathlib import Path

DISTINCT = 10.0  # CIEDE2000 below this: treat the pair as not distinguishable by colour alone (a judgement)

STATUS = ("info", "success", "warning", "danger", "important")

# (foreground token, background, minimum ratio, what it is[, only these themes]). Tokens are read as
# --<theme>-<name>. A background that starts with # is a literal colour: mdBook's own code-block backgrounds,
# which the palette does not set. Status colours are checked on `bg` only, because that is where the book draws
# them. They are not checked on `surface` or `surface-2`, and some fall below 4.5:1 there.
PAIRS = [
    ("fg", "bg", 4.5, "body text"),
    ("fg", "surface", 4.5, "text on the sidebar and panels"),
    ("fg", "surface-2", 4.5, "text on quotes and table headers"),
    ("fg", "mark", 4.5, "text on the search highlight"),
    ("link", "mark", 4.5, "links on the footnote highlight"),
    ("fg-muted", "bg", 4.5, "secondary text"),
    ("fg-muted", "surface", 4.5, "secondary text on panels"),
    ("link", "bg", 4.5, "links"),
    ("link", "surface", 4.5, "links on panels"),
    ("accent-text", "bg", 4.5, "accent used as text (the active item)"),
    ("accent-text", "surface", 4.5, "accent used as text on panels"),
    ("accent-text", "surface-2", 4.5, "accent text on a quote or table-header background"),
    ("accent-text", "#f6f7f6", 4.5, "inline code on mdBook's light code background", ("light",)),
    ("accent-text", "#1d1f21", 4.5, "inline code on mdBook's coal and navy code background", ("dark",)),
    ("accent-text", "#191f26", 4.5, "inline code on mdBook's ayu code background", ("dark",)),
    ("on-accent", "accent", 4.5, "text on an accent fill (not drawn by the book yet)"),
    ("accent", "bg", 3.0, "accent as a non-text marker"),
] + [(f"status-{s}", "bg", 4.5, f"{s} title text") for s in STATUS]

# Machado, Oliveira, Fernandes 2009, severity 1.0, applied to LINEAR sRGB.
CVD = {
    "protan": [[0.152286, 1.052583, -0.204868], [0.114503, 0.786281, 0.099216], [-0.003882, -0.048116, 1.051998]],
    "deutan": [[0.367322, 0.860646, -0.227968], [0.280085, 0.672501, 0.047413], [-0.011820, 0.042940, 0.968881]],
    "tritan": [[1.255528, -0.076749, -0.178779], [-0.078411, 0.930809, 0.147602], [0.004733, 0.691367, 0.303900]],
}


# ---------------------------------------------------------------------------------------------------------
# colour maths
# ---------------------------------------------------------------------------------------------------------


def hex_to_rgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))


def rgb_to_hex(rgb):
    return "#%02x%02x%02x" % tuple(max(0, min(255, round(c))) for c in rgb)


def to_linear(c8):
    c = c8 / 255.0
    return c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4  # the constant WCAG 2 states


def from_linear(c):
    c = max(0.0, min(1.0, c))
    return 255.0 * (12.92 * c if c <= 0.0031308 else 1.055 * c ** (1 / 2.4) - 0.055)


def luminance(h):
    r, g, b = (to_linear(c) for c in hex_to_rgb(h))
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(a, b):
    la, lb = luminance(a), luminance(b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


def simulate_rgb(h, matrix):
    """The colour as seen with the given deficiency, as unrounded 0-255 floats (rounding to a hex byte would
    shift a CIEDE2000 distance by up to about 0.2)."""
    lin = [to_linear(c) for c in hex_to_rgb(h)]
    out = [sum(matrix[i][j] * lin[j] for j in range(3)) for i in range(3)]
    return tuple(from_linear(c) for c in out)


def simulate(h, matrix):
    return rgb_to_hex(simulate_rgb(h, matrix))


def hex_to_lab(h):
    return rgb_to_lab(hex_to_rgb(h))


def rgb_to_lab(rgb):
    r, g, b = (to_linear(c) for c in rgb)
    x = 0.4124564 * r + 0.3575761 * g + 0.1804375 * b
    y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b
    z = 0.0193339 * r + 0.1191920 * g + 0.9503041 * b
    xn, yn, zn = 0.95047, 1.0, 1.08883

    def f(t):
        d = 6 / 29
        return t ** (1 / 3) if t > d ** 3 else t / (3 * d * d) + 4 / 29

    fx, fy, fz = f(x / xn), f(y / yn), f(z / zn)
    return 116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)


def delta_e2000(lab1, lab2):
    """CIEDE2000 (Sharma, Wu, Dalal 2005)."""
    l1, a1, b1 = lab1
    l2, a2, b2 = lab2
    c1, c2 = math.hypot(a1, b1), math.hypot(a2, b2)
    cb = (c1 + c2) / 2
    g = 0.5 * (1 - math.sqrt(cb ** 7 / (cb ** 7 + 25 ** 7)))
    a1p, a2p = (1 + g) * a1, (1 + g) * a2
    c1p, c2p = math.hypot(a1p, b1), math.hypot(a2p, b2)
    h1p = math.degrees(math.atan2(b1, a1p)) % 360 if c1p else 0.0
    h2p = math.degrees(math.atan2(b2, a2p)) % 360 if c2p else 0.0
    dl, dc = l2 - l1, c2p - c1p
    if c1p * c2p == 0:
        dh = 0.0
    else:
        dh = h2p - h1p
        if dh > 180:
            dh -= 360
        elif dh < -180:
            dh += 360
    dhp = 2 * math.sqrt(c1p * c2p) * math.sin(math.radians(dh / 2))
    lbp, cbp = (l1 + l2) / 2, (c1p + c2p) / 2
    if c1p * c2p == 0:
        hbp = h1p + h2p
    elif abs(h1p - h2p) <= 180:
        hbp = (h1p + h2p) / 2
    elif h1p + h2p < 360:
        hbp = (h1p + h2p + 360) / 2
    else:
        hbp = (h1p + h2p - 360) / 2
    t = (1 - 0.17 * math.cos(math.radians(hbp - 30)) + 0.24 * math.cos(math.radians(2 * hbp))
         + 0.32 * math.cos(math.radians(3 * hbp + 6)) - 0.20 * math.cos(math.radians(4 * hbp - 63)))
    dtheta = 30 * math.exp(-(((hbp - 275) / 25) ** 2))
    rc = 2 * math.sqrt(cbp ** 7 / (cbp ** 7 + 25 ** 7))
    sl = 1 + 0.015 * (lbp - 50) ** 2 / math.sqrt(20 + (lbp - 50) ** 2)
    sc = 1 + 0.045 * cbp
    sh = 1 + 0.015 * cbp * t
    rt = -math.sin(math.radians(2 * dtheta)) * rc
    return math.sqrt((dl / sl) ** 2 + (dc / sc) ** 2 + (dhp / sh) ** 2 + rt * (dc / sc) * (dhp / sh))


# ---------------------------------------------------------------------------------------------------------
# tokens
# ---------------------------------------------------------------------------------------------------------


def parse_tokens(css):
    css = re.sub(r"/\*.*?\*/", "", css, flags=re.S)
    return {m.group(1): m.group(2).strip() for m in re.finditer(r"--([a-z0-9-]+)\s*:\s*([^;]+);", css)}


def resolve(tokens, name, seen=()):
    if name in seen:
        raise ValueError(f"circular token --{name}")
    value = tokens.get(name)
    if value is None:
        raise KeyError(name)
    m = re.fullmatch(r"var\(--([a-z0-9-]+)\)", value)
    if m:
        return resolve(tokens, m.group(1), seen + (name,))
    if not re.fullmatch(r"#[0-9a-fA-F]{6}", value):
        raise ValueError(f"--{name}: {value!r} is neither #rrggbb nor var(--token)")
    return value.lower()


def evaluate_pairs(tokens, themes=("light", "dark")):
    """Return (rows, failures). A missing token is a failure, not a skip."""
    rows, failures = [], []
    for theme in themes:
        for fg, bg, need, what, *only in PAIRS:
            if only and theme not in only[0]:
                continue
            try:
                a = resolve(tokens, f"{theme}-{fg}")
                b = bg.lower() if bg.startswith("#") else resolve(tokens, f"{theme}-{bg}")
            except (KeyError, ValueError) as e:
                failures.append(f"{theme}: {fg} on {bg}: cannot resolve {e}")
                continue
            ratio = contrast(a, b)
            ok = ratio >= need
            rows.append((theme, fg, bg, a, b, ratio, need, ok, what))
            if not ok:
                failures.append(f"{theme}: {fg} {a} on {bg} {b} is {ratio:.2f}:1, needs {need}:1 ({what})")
    return rows, failures


def status_distance(tokens, theme):
    colours = {}
    for s in STATUS:
        colours[s] = resolve(tokens, f"{theme}-status-{s}")
    out = []
    names = list(STATUS)
    for i, a in enumerate(names):
        for b in names[i + 1:]:
            worst, which = 1e9, ""
            for sim in ("normal",) + tuple(CVD):
                if sim == "normal":
                    la, lb = hex_to_lab(colours[a]), hex_to_lab(colours[b])
                else:
                    la = rgb_to_lab(simulate_rgb(colours[a], CVD[sim]))
                    lb = rgb_to_lab(simulate_rgb(colours[b], CVD[sim]))
                d = delta_e2000(la, lb)
                if d < worst:
                    worst, which = d, sim
            out.append((a, b, worst, which))
    return colours, out


# ---------------------------------------------------------------------------------------------------------
# self-tests: known answers, and one control that must fail
# ---------------------------------------------------------------------------------------------------------


def self_test():
    errors = []

    def expect(name, got, want, tol):
        if not abs(got - want) <= tol:  # written this way so that a NaN fails instead of passing
            errors.append(f"{name}: got {got}, want {want}")

    expect("black on white", contrast("#000000", "#ffffff"), 21.0, 1e-9)
    if not contrast("#767676", "#ffffff") >= 4.5:
        errors.append("#767676 on white must pass 4.5:1 (it is 4.54:1)")
    if contrast("#777777", "#ffffff") >= 4.5:
        errors.append("#777777 on white must FAIL 4.5:1 (it is 4.48:1)")
    sharma = [
        ((50.0, 2.6772, -79.7751), (50.0, 0.0, -82.7485), 2.0425),
        ((50.0, 3.1571, -77.2803), (50.0, 0.0, -82.7485), 2.8615),
        ((50.0, 2.8361, -74.0200), (50.0, 0.0, -82.7485), 3.4412),
        ((50.0, 2.5, 0.0), (73.0, 25.0, -18.0), 27.1492),
    ]
    for i, (p, q, want) in enumerate(sharma, 1):
        expect(f"CIEDE2000 Sharma pair {i}", delta_e2000(p, q), want, 1e-4)
    # The pairs above never reach CIEDE2000's hue-wrap branches. These two hues straddle 0/360 (about 354 and
    # 6 degrees), so they are close, and the distance must come out small and symmetric.
    near = delta_e2000((50.0, 10.0, -1.0), (50.0, 10.0, 1.0))
    if not 0 < near < 2.0:
        errors.append(f"CIEDE2000 across the 0/360 hue boundary should be small, got {near}")
    expect("CIEDE2000 symmetry", near, delta_e2000((50.0, 10.0, 1.0), (50.0, 10.0, -1.0)), 1e-9)
    for name, m in CVD.items():
        for row in m:
            expect(f"{name} row sums to 1", sum(row), 1.0, 1e-4)
        if simulate("#808080", m) != "#808080":
            errors.append(f"{name} must leave a grey unchanged, got {simulate('#808080', m)}")
    # control: a palette that is wrong must be reported as wrong
    _, failures = evaluate_pairs({"light-fg": "#777777", "light-bg": "#ffffff"}, themes=("light",))
    if not any("fg #777777 on bg #ffffff" in f for f in failures):
        errors.append("control: grey #777777 on white was not reported as failing 4.5:1")
    return errors


def main(argv):
    css_path = Path(__file__).with_name("colors.css")
    if "--css" in argv:
        i = argv.index("--css")
        if i + 1 >= len(argv):
            print("--css needs a path")
            return 3
        css_path = Path(argv[i + 1])

    errors = self_test()
    if errors:
        print("SELF-TEST FAILED, refusing to report on the palette:")
        for e in errors:
            print("  " + e)
        return 2
    print("self-test: ok (contrast, CIEDE2000, colour-blindness matrices, and a must-fail control)")

    try:
        tokens = parse_tokens(css_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError) as e:
        print(f"cannot read the token file: {e}")
        return 3
    rows, failures = evaluate_pairs(tokens)

    print(f"\ncontrast, WCAG 2 ({css_path.name})")
    print(f"{'theme':6} {'text':22} {'on':10} {'ratio':>7} {'need':>5}  what")
    for theme, fg, bg, a, b, ratio, need, ok, what in rows:
        print(f"{theme:6} {fg:22} {bg:10} {ratio:7.2f} {need:5.1f}  {'ok  ' if ok else 'FAIL'} {what}")

    print(f"\nstatus colours under colour blindness (CIEDE2000, worst of normal/protan/deutan/tritan; below {DISTINCT:g} = not told apart by colour alone)")
    for theme in ("light", "dark"):
        try:
            colours, dist = status_distance(tokens, theme)
        except (KeyError, ValueError) as e:
            failures.append(f"{theme}: status colours: cannot resolve {e}")
            continue
        print(f"{theme}: " + "  ".join(f"{s}={c}" for s, c in colours.items()))
        for a, b, d, which in dist:
            flag = "needs a label or icon" if d < DISTINCT else ""
            print(f"  {a:9} vs {b:9} {d:6.1f} ({which:6}) {flag}")

    if failures:
        print(f"\nFAILED: {len(failures)} problem(s)")
        for f in failures:
            print("  " + f)
        return 1
    print("\ncontrast: every pair passes")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
