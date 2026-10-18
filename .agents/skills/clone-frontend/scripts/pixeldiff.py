#!/usr/bin/env -S uv run --offline --script
# SPDX-License-Identifier: AGPL-3.0-only
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""pixeldiff.py - offline pixel-diff gate for the clone-frontend skill.

Python standard library only (the PEP 723 block declares no dependencies; with
`uv run --offline` nothing is downloaded, so a local Python 3.10+ is required).
`compare` reads the two PNG files named on the command line, plus the first 8
bytes of an existing --out (to confirm it is a PNG), and writes only the optional
--out file. `selftest` writes synthetic PNGs to a temporary directory and deletes
it. No network, no subprocess. Deterministic: same inputs and flags give the same
verdict and report.

  uv run --offline --script scripts/pixeldiff.py compare ORIGINAL.png PORT.png \
      [--tolerance-pct 0.5] [--channel-delta 8] [--mask X,Y,W,H ...] \
      [--max-masked-pct 10] [--min-colors 8] [--out diff.png]
  uv run --offline --script scripts/pixeldiff.py selftest

Exit codes: 0 gate held; 1 gate failed (over tolerance, size mismatch, a blank
image, or masks covering more than --max-masked-pct); 2 usage error, or an
unreadable/unsupported/oversized PNG (8-bit, non-interlaced only; above 50 million
pixels or above 20,000 on either side), or --out naming an input (same file, hard
links included), a symbolic link (dangling ones too) or an existing file that is
not a PNG. Alpha is ignored; screenshots are expected to be opaque.

Every setting that weakens or departs from the defaults (--min-colors below 8,
--max-masked-pct above 10, --tolerance-pct above 0.5, --channel-delta other than 8,
any --mask) adds one `OVERRIDE ...` line to the report so the departure is visible
in the transcript. --min-colors below 8, --max-masked-pct above 10, --channel-delta
above 32 and --tolerance-pct above 5 need the user's approval.
"""
import argparse
import os
import struct
import sys
import tempfile
import zlib
from pathlib import Path

SIG = b"\x89PNG\r\n\x1a\n"
CHANNELS = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}


class PngError(Exception):
    pass


def _paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    return a if pa <= pb and pa <= pc else (b if pb <= pc else c)


def _unfilter(raw, w, h, bpp):
    stride, prev, rows, pos = w * bpp, bytearray(w * bpp), [], 0
    for _ in range(h):
        ft, cur = raw[pos], bytearray(raw[pos + 1:pos + 1 + stride])
        pos += 1 + stride
        if ft == 1:
            for i in range(bpp, stride):
                cur[i] = (cur[i] + cur[i - bpp]) & 255
        elif ft == 2:
            for i in range(stride):
                cur[i] = (cur[i] + prev[i]) & 255
        elif ft == 3:
            for i in range(stride):
                left = cur[i - bpp] if i >= bpp else 0
                cur[i] = (cur[i] + ((left + prev[i]) >> 1)) & 255
        elif ft == 4:
            for i in range(stride):
                left = cur[i - bpp] if i >= bpp else 0
                ul = prev[i - bpp] if i >= bpp else 0
                cur[i] = (cur[i] + _paeth(left, prev[i], ul)) & 255
        elif ft != 0:
            raise PngError(f"bad filter type {ft}")
        rows.append(cur)
        prev = cur
    return rows


MAX_PIXELS = 50_000_000  # area cap: a 7000x7000 screenshot is already far beyond a UI capture
MAX_DIM = 20_000  # side cap: the area cap alone lets a 1 x 50,000,000 image through (50 million row objects)


def _check_size(path, w, h):
    """Reject a zero-sized image, one above MAX_PIXELS, or one above MAX_DIM on either side (before any allocation)."""
    if w == 0 or h == 0 or w * h > MAX_PIXELS or max(w, h) > MAX_DIM:
        raise PngError(f"{path}: image size {w}x{h} is zero, above {MAX_PIXELS} pixels or above {MAX_DIM} on a side")


def read_png(path):
    """Return (width, height, rows) where each row is RGB bytes (3 per pixel).

    Every malformed-input path raises PngError (never a traceback)."""
    try:
        return _read_png(path)
    except (struct.error, IndexError, ValueError, zlib.error) as e:
        raise PngError(f"{path}: corrupt PNG ({e.__class__.__name__}: {e})")


def _read_png(path):
    data = Path(path).read_bytes()
    if data[:8] != SIG:
        raise PngError(f"{path}: not a PNG")
    pos, idat, hdr, plte = 8, [], None, b""
    while pos + 8 <= len(data):
        n, typ = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        pos += 12 + n
        if typ == b"IHDR":
            hdr = struct.unpack(">IIBBBBB", body)
        elif typ == b"PLTE":
            plte = body
        elif typ == b"IDAT":
            idat.append(body)
        elif typ == b"IEND":
            break
    if hdr is None:
        raise PngError(f"{path}: no IHDR chunk")
    w, h, depth, ctype, _, _, interlace = hdr
    if depth != 8 or interlace or ctype not in CHANNELS:
        raise PngError(f"{path}: unsupported PNG (bit depth {depth}, colour type "
                       f"{ctype}, interlace {interlace}); need 8-bit, non-interlaced")
    _check_size(path, w, h)
    bpp = CHANNELS[ctype]
    expected = h * (w * bpp + 1)
    raw = zlib.decompressobj().decompress(b"".join(idat), expected + 1)  # bounded: no decompression bomb
    if len(raw) != expected:
        raise PngError(f"{path}: pixel data length does not match header")
    out = []
    pal = [bytes(plte[i * 3 + c] if i * 3 + c < len(plte) else 0 for i in range(256))
           for c in range(3)] if ctype == 3 else None
    for row in _unfilter(raw, w, h, bpp):
        rgb = bytearray(w * 3)
        if ctype in (2, 6):
            for c in range(3):
                rgb[c::3] = row[c::bpp]
        elif ctype in (0, 4):
            for c in range(3):
                rgb[c::3] = row[0::bpp]
        else:
            for c in range(3):
                rgb[c::3] = bytes(row).translate(pal[c])
        out.append(bytes(rgb))
    return w, h, out


def write_png(path, w, h, rows, ctype=2, filters=(0,)):
    """Minimal PNG writer. rows are RGB bytes; filters rotate per row.

    Colour types: 0 gray (red channel), 2 RGB, 3 palette (<= 256 colours), 4 gray+alpha, 6 RGBA."""
    bpp, prev, body = CHANNELS[ctype], bytearray(w * CHANNELS[ctype]), bytearray()
    palette = []
    if ctype == 3:
        palette = sorted({tuple(r[i:i + 3]) for r in rows for i in range(0, len(r), 3)})
        if len(palette) > 256:
            raise ValueError("palette images need at most 256 colours")
    index = {c: i for i, c in enumerate(palette)}
    for y, rgb in enumerate(rows):
        if ctype == 2:
            cur = bytearray(rgb)
        elif ctype == 6:
            cur = bytearray(w * 4)
            for c in range(3):
                cur[c::4] = rgb[c::3]
            cur[3::4] = b"\xff" * w
        elif ctype == 0:
            cur = bytearray(rgb[0::3])
        elif ctype == 4:
            cur = bytearray(w * 2)
            cur[0::2] = rgb[0::3]
            cur[1::2] = b"\xff" * w
        else:
            cur = bytearray(index[tuple(rgb[i:i + 3])] for i in range(0, len(rgb), 3))
        ft, enc = filters[y % len(filters)], bytearray(len(cur))
        for i in range(len(cur)):
            left = cur[i - bpp] if i >= bpp else 0
            up = prev[i]
            ul = prev[i - bpp] if i >= bpp else 0
            base = (0, left, up, (left + up) >> 1, _paeth(left, up, ul))[ft]
            enc[i] = (cur[i] - base) & 255
        body += bytes([ft]) + enc
        prev = cur

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))

    plte = chunk(b"PLTE", bytes(v for c in palette for v in c)) if ctype == 3 else b""
    Path(path).write_bytes(SIG + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, ctype, 0, 0, 0)) + plte
                           + chunk(b"IDAT", zlib.compress(bytes(body))) + chunk(b"IEND", b""))


def colour_count(rows, cap=256):
    seen = set()
    for r in rows:
        seen.update(zip(r[0::3], r[1::3], r[2::3]))
        if len(seen) >= cap:
            break
    return len(seen)


def parse_mask(s):
    try:
        x, y, mw, mh = (int(v) for v in s.split(","))
    except ValueError:
        raise PngError(f"bad --mask {s!r}; expected X,Y,W,H integers")
    return x, y, mw, mh


def _same_file(a, b):
    """True when both names refer to one file (hard links and case variants included)."""
    try:
        if os.path.exists(a) and os.path.exists(b):
            return os.path.samefile(a, b)
    except OSError:
        pass
    return Path(a).resolve() == Path(b).resolve()


def _is_png_file(p):
    try:
        with open(p, "rb") as fh:
            return fh.read(8) == SIG
    except OSError:
        return False


def overrides(tol_pct, delta, min_colors, max_masked_pct, masks):
    """One line per gate setting that weakens or departs from the defaults."""
    o = []
    if min_colors < 8:
        o.append(f"OVERRIDE min_colors={min_colors} (default 8) weakens the blank-image guard; needs the user's approval")
    if max_masked_pct > 10:
        o.append(f"OVERRIDE max_masked_pct={max_masked_pct} (default 10) weakens the mask cap; needs the user's approval")
    if tol_pct > 5:
        o.append(f"OVERRIDE tolerance_pct={tol_pct} (default 0.5) is above 5 and can hide a wrong page; needs the user's approval")
    elif tol_pct > 0.5:
        o.append(f"OVERRIDE tolerance_pct={tol_pct} (default 0.5): record the calibration and reason in clone-report.md")
    if delta > 32:
        o.append(f"OVERRIDE channel_delta={delta} (default 8) is above 32 and can disable the comparison; needs the user's approval")
    elif delta != 8:
        o.append(f"OVERRIDE channel_delta={delta} (default 8): record the reason in clone-report.md")
    if masks:
        o.append(f"OVERRIDE masks={len(masks)}: each needs a row with a reason in clone-report.md")
    return o


def compare(a_path, b_path, tol_pct=0.5, delta=8, masks=(), min_colors=8, out=None, max_masked_pct=10.0):
    """Return (exit_code, report_lines). Results (exit 0 or 1) end with one OVERRIDE line per weakened setting."""
    code, lines = _compare(a_path, b_path, tol_pct, delta, masks, min_colors, out, max_masked_pct)
    if code in (0, 1):
        lines = lines + overrides(tol_pct, delta, min_colors, max_masked_pct, masks)
    return code, lines


def _compare(a_path, b_path, tol_pct, delta, masks, min_colors, out, max_masked_pct):
    if _same_file(a_path, b_path):
        return 2, ["error: both arguments are the same file; that compares nothing"]
    if out and os.path.islink(out):  # true for dangling links too; writing would create the link's target
        return 2, ["error: --out is a symbolic link; refusing to write through it"]
    if out and (_same_file(out, a_path) or _same_file(out, b_path)):
        return 2, ["error: --out names an input file; refusing to overwrite it"]
    if out and os.path.lexists(out) and not (os.path.isfile(out) and _is_png_file(out)):
        return 2, ["error: --out exists and is not a PNG file; refusing to overwrite it"]
    try:
        wa, ha, ra = read_png(a_path)
        wb, hb, rb = read_png(b_path)
    except (PngError, OSError) as e:
        return 2, [f"error: {e}"]
    lines = []
    if (wa, ha) != (wb, hb):
        return 1, [f"RESULT FAIL size_mismatch original={wa}x{ha} port={wb}x{hb} "
                   "(fix viewport/DPR; this tool never resizes)"]
    for name, rows in (("original", ra), ("port", rb)):
        n = colour_count(rows)
        if n < min_colors:
            return 1, [f"RESULT FAIL blank_image {name} has {n} distinct colours "
                       f"(< {min_colors}); the capture is blank or an error page"]
    diff_rows, hot, bbox, diffs, masked = [], {}, [wa, ha, -1, -1], 0, 0
    for y in range(ha):
        a, b = bytearray(ra[y]), bytearray(rb[y])
        spans = sorted((max(mx, 0), min(mx + mw, wa)) for mx, my, mw, mh in masks
                       if my <= y < my + mh and min(mx + mw, wa) > max(mx, 0))
        end = 0
        for x0, x1 in spans:  # union of the spans on this row, so overlapping masks count once
            x0 = max(x0, end)
            if x1 > x0:
                masked += x1 - x0
                b[x0 * 3:x1 * 3] = a[x0 * 3:x1 * 3]
                end = x1
        xs = []
        if a != b:
            for i in range(0, len(a), 3):
                if (abs(a[i] - b[i]) > delta or abs(a[i + 1] - b[i + 1]) > delta
                        or abs(a[i + 2] - b[i + 2]) > delta):
                    xs.append(i // 3)
        diff_rows.append(xs)
        for x in xs:
            diffs += 1
            hot[(x // 64, y // 64)] = hot.get((x // 64, y // 64), 0) + 1
            bbox = [min(bbox[0], x), min(bbox[1], y), max(bbox[2], x), max(bbox[3], y)]
    pct = 100.0 * diffs / (wa * ha)
    masked_pct = 100.0 * masked / (wa * ha)
    verdict = "PASS" if pct <= tol_pct and masked_pct <= max_masked_pct else "FAIL"
    lines.append(f"RESULT {verdict} diff_pct={pct:.4f} diff_pixels={diffs} total={wa * ha} "
                 f"tolerance_pct={tol_pct} channel_delta={delta} masked_pixels={masked} "
                 f"masked_pct={masked_pct:.2f} max_masked_pct={max_masked_pct} "
                 f"bbox={','.join(map(str, bbox)) if diffs else 'none'}")
    if masked_pct > max_masked_pct:
        lines.append(f"masks_too_large: masks cover {masked_pct:.2f}% of the image (limit {max_masked_pct}%); "
                     "a mask that large hides the page, not a dynamic region")
    for (cx, cy), n in sorted(hot.items(), key=lambda kv: -kv[1])[:5]:
        lines.append(f"hot_cell x={cx * 64} y={cy * 64} size=64 diff_pixels={n}")
    if out:
        lighten = bytes(v // 3 + 170 for v in range(256))
        rows = []
        for y in range(ha):
            row = bytearray(ra[y].translate(lighten))
            for x in diff_rows[y]:
                row[x * 3:x * 3 + 3] = b"\xff\x00\x00"
            rows.append(bytes(row))
        write_png(out, wa, ha, rows)
        lines.append(f"diff_image {out}")
    return (0 if verdict == "PASS" else 1), lines


def _scene(w=200, h=120):
    rows = []
    for y in range(h):
        r = bytearray()
        for x in range(w):
            r += bytes(((x * 255) // w, (y * 255) // h, ((x ^ y) * 7) & 255))
        rows.append(bytes(r))
    return rows


def _paint(rows, x0, y0, rw, rh, rgb):
    out = [bytearray(r) for r in rows]
    for y in range(y0, y0 + rh):
        out[y][x0 * 3:(x0 + rw) * 3] = bytes(rgb) * rw
    return [bytes(r) for r in out]


def _shift_channel(rows, x0, y0, rw, rh, c, amount=100):
    out = [bytearray(r) for r in rows]
    for y in range(y0, y0 + rh):
        for x in range(x0, x0 + rw):
            out[y][x * 3 + c] = (out[y][x * 3 + c] + amount) & 255
    return [bytes(r) for r in out]


def _blocks(w=200, h=120):
    """4x4 grid of 16 distinct flat colours: small enough for a palette image."""
    rows = []
    for y in range(h):
        r = bytearray()
        for x in range(w):
            i, j = x * 4 // w, y * 4 // h
            r += bytes(((i * 60 + 20) % 256, (j * 90 + 40) % 256, (i * j * 40 + 10) % 256))
        rows.append(bytes(r))
    return rows


def _raw_png(path, w, h, depth=8, ctype=2, interlace=0, idat=b"", ihdr=None):
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))

    head = ihdr if ihdr is not None else struct.pack(">IIBBBBB", w, h, depth, ctype, 0, 0, interlace)
    Path(path).write_bytes(SIG + chunk(b"IHDR", head) + chunk(b"IDAT", idat) + chunk(b"IEND", b""))
    return str(path)


def selftest():
    w, h, ok = 200, 120, True
    base = _scene(w, h)
    blocks = _blocks(w, h)
    with tempfile.TemporaryDirectory() as d:
        def put(name, rows, ctype=2, filters=(0, 1, 2, 3, 4)):
            p = f"{d}/{name}.png"
            write_png(p, w, h, rows, ctype, filters)
            return p

        noise = [bytes(min(v + 3, 255) if i % 97 == 0 else v for i, v in enumerate(r)) for r in base]
        big = _paint(base, 20, 20, 40, 20, (255, 0, 255))          # 800 px = 3.33%
        small = _paint(base, 20, 20, 10, 3, (255, 0, 255))         # 30 px = 0.125%
        flat = [bytes([200]) * (w * 3)] * h
        fits8 = zlib.compress(bytes(h * (w * 3 + 1)))  # exactly the size an 8-bit RGB reading expects
        a = put("a", base)
        cases = [
            ("identical content, other filters/colour type -> PASS", compare(a, put("a_rgba", base, 6, (4, 3, 2, 1, 0))), 0),
            ("gray-type decode path runs -> PASS vs itself", compare(put("g1", base, 0), put("g2", base, 0)), 0),
            ("palette (type 3) decodes like RGB -> PASS", compare(put("p1", blocks), put("p2", blocks, 3)), 0),
            ("palette vs RGB with one block altered -> FAIL",
             compare(put("p3", _paint(blocks, 0, 0, 50, 30, (1, 2, 3))), put("p4", blocks, 3)), 1),
            ("gray+alpha (type 4) decodes like gray -> PASS", compare(put("ga1", base, 0), put("ga2", base, 4)), 0),
            ("gray+alpha vs gray with a changed region -> FAIL",
             compare(put("ga3", _shift_channel(base, 20, 20, 40, 20, 0), 0), put("ga4", base, 4)), 1),
            ("sub-threshold noise -> PASS", compare(a, put("noise", noise)), 0),
            ("MUST-FAIL CONTROL: altered region over tolerance -> FAIL", compare(a, put("big", big)), 1),
            ("small change inside 0.5% tolerance -> PASS", compare(a, put("small", small)), 0),
            ("same small change, 0.05% tolerance -> FAIL", compare(a, put("small2", small), tol_pct=0.05), 1),
            ("altered region covered by a recorded mask -> PASS", compare(a, put("big2", big), masks=[(20, 20, 40, 20)]), 0),
            ("mask on the same rows but other columns does not hide the change -> FAIL", compare(a, put("big3", big), masks=[(150, 20, 40, 20)]), 1),
            ("whole-image mask hides everything -> FAIL (mask cap)", compare(a, put("big4", big), masks=[(0, 0, w, h)]), 1),
            ("whole-image mask over identical images -> FAIL (mask cap)", compare(a, put("same", base), masks=[(0, 0, w, h)]), 1),
            ("overlapping masks are counted once (1000 px)",
             compare(a, put("same2", base), masks=[(20, 20, 40, 20), (30, 20, 40, 20)]), 0,
             lambda lines: "masked_pixels=1000 " in lines[0]),
            ("blank vs blank -> FAIL (blank guard)", compare(put("f1", flat), put("f2", flat)), 1),
            ("same file twice -> usage error 2", compare(a, a), 2),
            ("--out naming an input -> usage error 2", compare(a, put("o1", base), out=a), 2),
            ("missing input file -> exit 2", compare(a, f"{d}/does-not-exist.png"), 2),
            ("not a PNG -> exit 2", compare(a, _raw_text(f"{d}/t.png", b"hello")), 2),
            ("truncated IHDR -> exit 2", compare(a, _raw_png(f"{d}/ih.png", 0, 0, ihdr=b"\x00\x00")), 2),
            ("16-bit PNG with 8-bit-sized data -> exit 2 (depth check, not length)",
             compare(a, _raw_png(f"{d}/d16.png", w, h, depth=16, idat=fits8)), 2),
            ("interlaced PNG with 8-bit-sized data -> exit 2",
             compare(a, _raw_png(f"{d}/il.png", w, h, interlace=1, idat=fits8)), 2),
            ("zero-width PNG -> exit 2", compare(a, _raw_png(f"{d}/z.png", 0, h)), 2),
            ("oversized dimensions -> exit 2 (cap, nothing allocated)", compare(a, _raw_png(f"{d}/hu.png", 100000, 100000)), 2),
            ("decompression bomb in a tiny image -> exit 2",
             compare(a, _raw_png(f"{d}/bomb.png", 2, 2, idat=zlib.compress(b"\x00" * 5_000_000))), 2),
        ]
        no_override = lambda lines: not any("OVERRIDE" in line for line in lines)  # noqa: E731
        has = lambda text: (lambda lines: any(text in line for line in lines))  # noqa: E731
        prev_png = put("prev", base)
        junk = _raw_text(f"{d}/notes.txt", b"hello")
        link = f"{d}/a_link.png"
        try:
            os.link(a, link)
        except OSError:
            link = None
        if link:
            cases += [
                ("hard link to the same file -> usage error 2", compare(a, link), 2),
                ("--out hard-linked to an input -> usage error 2", compare(a, put("ol", big), out=link), 2),
            ]
        def size_ok(sw, sh):
            try:
                _check_size("x", sw, sh)
                return True
            except PngError:
                return False

        def zeros(sw, sh):  # valid pixel data for an all-zero RGB image, so only the size cap can reject it
            return zlib.compress(bytes(sh * (sw * 3 + 1)))

        wide_row = [bytes((i * 7) % 256 for i in range(20_000 * 3))]
        tall_rows = [bytes(((i * 7) % 256, (i * 13) % 256, (i * 29) % 256)) for i in range(20_000)]
        write_png(f"{d}/w20k_a.png", 20_000, 1, wide_row, 2, (0,))
        write_png(f"{d}/w20k_b.png", 20_000, 1, wide_row, 2, (0,))
        write_png(f"{d}/t20k_a.png", 1, 20_000, tall_rows, 2, (0,))
        write_png(f"{d}/t20k_b.png", 1, 20_000, tall_rows, 2, (0,))
        cases += [
            ("size check accepts 1x1, 20000x1, 1x20000, 20000x2500 (exactly 50,000,000 pixels) and 7000x7000",
             (0 if all(size_ok(*s) for s in [(1, 1), (20_000, 1), (1, 20_000), (20_000, 2_500), (7_000, 7_000)]) else 1, []), 0),
            ("size check rejects zero sides, 20,001 on either side, 1 x 50,000,000 (area at the cap), "
             "10000x5001 (area above it) and 100000x100000",
             (0 if not any(size_ok(*s) for s in [(0, 5), (5, 0), (20_001, 1), (1, 20_001), (1, 50_000_000),
                                                 (50_000_000, 1), (10_000, 5_001), (100_000, 100_000)]) else 1, []), 0),
            ("1 x 50,000,000 (area exactly at the cap) with no data -> exit 2",
             compare(a, _raw_png(f"{d}/tall.png", 1, 50_000_000)), 2),
            ("20,001 x 1 with valid data -> exit 2 (side cap, not a data error)",
             compare(a, _raw_png(f"{d}/w20001.png", 20_001, 1, idat=zeros(20_001, 1))), 2),
            ("1 x 20,001 with valid data -> exit 2 (side cap, not a data error)",
             compare(a, _raw_png(f"{d}/t20001.png", 1, 20_001, idat=zeros(1, 20_001))), 2),
            ("20,000 x 1 is still read (identical pair -> PASS)",
             compare(f"{d}/w20k_a.png", f"{d}/w20k_b.png"), 0),
            ("1 x 20,000 is still read (identical pair -> PASS)",
             compare(f"{d}/t20k_a.png", f"{d}/t20k_b.png"), 0),
        ]
        try:
            os.symlink(prev_png, f"{d}/out_link.png")
            os.symlink(f"{d}/no_such_target.png", f"{d}/dangling.png")
            symlinks = True
        except OSError:
            symlinks = False
        if symlinks:
            cases += [
                ("--out through a symbolic link to a PNG -> usage error 2",
                 compare(a, put("os1", big), out=f"{d}/out_link.png"), 2),
                ("--out through a dangling symbolic link -> usage error 2, target not created",
                 compare(a, put("os2", big), out=f"{d}/dangling.png"), 2,
                 lambda lines: not os.path.exists(f"{d}/no_such_target.png")),
            ]
        cases += [
            ("--out naming an existing non-PNG file -> usage error 2", compare(a, put("on", big), out=junk), 2),
            ("--out naming an existing PNG is overwritten (diff image written)",
             compare(a, put("op", big), out=prev_png), 1, lambda lines: _is_png_file(prev_png)),
            ("defaults print no OVERRIDE line", compare(a, put("same4", base)), 0, no_override),
            ("masks print an OVERRIDE line", compare(a, put("same5", base), masks=[(0, 0, 10, 10)]), 0, has("OVERRIDE masks=1")),
            ("--min-colors 0 prints an OVERRIDE line", compare(put("f3", flat), put("f4", flat), min_colors=0), 0,
             has("OVERRIDE min_colors=0")),
            ("--max-masked-pct 100 prints an OVERRIDE line",
             compare(a, put("same6", base), masks=[(0, 0, w, h)], max_masked_pct=100.0), 0,
             has("OVERRIDE max_masked_pct=100")),
            ("--channel-delta 255 hides a real change; the OVERRIDE line says it needs approval",
             compare(a, put("big6", big), delta=255), 0, has("OVERRIDE channel_delta=255"),
             has("needs the user's approval")),
            ("tolerance 5 passes a 3.3% change; its OVERRIDE line asks only for a recorded reason",
             compare(a, put("big7", big), tol_pct=5.0), 0, has("OVERRIDE tolerance_pct=5.0"),
             lambda lines: not any("approval" in line for line in lines)),
            ("tolerance 20 prints an OVERRIDE line that needs approval",
             compare(a, put("big8", big), tol_pct=20.0), 0, has("OVERRIDE tolerance_pct=20.0"),
             has("needs the user's approval")),
            ("approval tiers: delta 32 and tolerance 5 need none; delta 33 and tolerance 5.5 do",
             (0 if not any("approval" in line for line in overrides(5.0, 32, 8, 10, ()))
              and any("approval" in line for line in overrides(0.5, 33, 8, 10, ()))
              and any("approval" in line for line in overrides(5.5, 8, 8, 10, ())) else 1, []), 0),
            ("channel-delta 16 and 0 print an OVERRIDE line that asks only for a recorded reason",
             (0 if all(any(f"channel_delta={v}" in line and "record the reason" in line
                           for line in overrides(0.5, v, 8, 10, ())) for v in (16, 0)) else 1, []), 0),
            ("approval tiers: min-colors 8 and max-masked-pct 10 need none; 7 and 10.5 do",
             (0 if not any("approval" in line for line in overrides(0.5, 8, 8, 10, ()))
              and any("approval" in line for line in overrides(0.5, 8, 7, 10, ()))
              and any("approval" in line for line in overrides(0.5, 8, 8, 10.5, ())) else 1, []), 0),
        ]
        for c, cname in enumerate("RGB"):
            shifted = _shift_channel(base, 20, 20, 40, 20, c)
            cases.append((f"MUST-FAIL: {cname}-channel-only change -> FAIL",
                          compare(a, put(f"ch{c}", shifted)), 1))
        write_png(f"{d}/short.png", w, 60, base[:60], 2, (0,))
        cases.append(("size mismatch -> FAIL", compare(a, f"{d}/short.png"), 1))
        for label, (code, lines), want, *extra in cases:
            good = code == want and all(chk(lines) for chk in extra)
            ok = ok and good
            print(("ok     " if good else "NOT OK ") + f"{label} (exit {code}, wanted {want})")
    print("SELFTEST " + ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


def _raw_text(path, data):
    Path(path).write_bytes(data)
    return str(path)


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("compare")
    c.add_argument("original")
    c.add_argument("port")
    c.add_argument("--tolerance-pct", type=float, default=0.5)
    c.add_argument("--channel-delta", type=int, default=8)
    c.add_argument("--mask", action="append", default=[], help="X,Y,W,H region to ignore (record why)")
    c.add_argument("--max-masked-pct", type=float, default=10.0,
                   help="fail if masks cover more than this share of the image (default 10)")
    c.add_argument("--min-colors", type=int, default=8)
    c.add_argument("--out", help="write a diff image here (must not be an input file)")
    sub.add_parser("selftest")
    args = ap.parse_args(argv)
    if args.cmd == "selftest":
        return selftest()
    try:
        code, lines = compare(args.original, args.port, args.tolerance_pct, args.channel_delta,
                              [parse_mask(m) for m in args.mask], args.min_colors, args.out,
                              args.max_masked_pct)
    except (PngError, OSError, zlib.error) as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    print("\n".join(lines))
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
