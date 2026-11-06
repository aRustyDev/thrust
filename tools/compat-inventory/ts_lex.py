# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""A small, dependency-free TypeScript/TSX lexer.

It is NOT a parser. It exists so the client scanner can ignore comments, treat string and template
literals as single tokens (with `${...}` parts kept), and match brackets reliably. Known limits:
  * JSX text containing an apostrophe is recovered from (an unterminated single-line string becomes a
    stray punct token), which can at worst hide a few tokens on that line;
  * regex-literal detection is heuristic (previous-token based).
"""

from __future__ import annotations

from dataclasses import dataclass

ID_START = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_$#")
ID_PART = ID_START | set("0123456789")
REGEX_PREV_PUNCT = set("(,=:[!&|?{};+-*%<>~^")
REGEX_PREV_KW = {"return", "typeof", "case", "in", "of", "delete", "void", "throw", "new", "else", "do"}
MULTI = ["...", "===", "!==", "=>", "?.", "??", "&&", "||", "==", "!=", "+=", "-=", "*=", "/=", "++", "--"]


@dataclass
class Tok:
    kind: str  # id | num | str | tpl | punct | regex
    text: str  # str: decoded body; tpl: raw inner text; others: source text
    pos: int
    end: int
    line: int
    parts: list | None = None  # tpl only: [("s", literal), ("e", expr_source)]

    def is_(self, kind: str, text: str | None = None) -> bool:
        return self.kind == kind and (text is None or self.text == text)

    def p(self, text: str) -> bool:
        return self.kind == "punct" and self.text == text

    def id(self, text: str | None = None) -> bool:
        return self.kind == "id" and (text is None or self.text == text)


def _read_template(src: str, i: int):
    """src[i] == '`'. Return (end_index_after_closing_tick, parts, line_increments)."""
    n = len(src)
    j = i + 1
    parts: list = []
    buf: list[str] = []
    nl = 0
    while j < n:
        c = src[j]
        if c == "\\":
            buf.append(src[j : j + 2])
            if j + 1 < n and src[j + 1] == "\n":
                nl += 1
            j += 2
            continue
        if c == "`":
            if buf:
                parts.append(("s", "".join(buf)))
            return j + 1, parts, nl
        if c == "$" and j + 1 < n and src[j + 1] == "{":
            if buf:
                parts.append(("s", "".join(buf)))
                buf = []
            depth, k = 1, j + 2
            while k < n and depth:
                ch = src[k]
                if ch == "{":
                    depth += 1
                elif ch == "}":
                    depth -= 1
                elif ch in "'\"":
                    q, k = ch, k + 1
                    while k < n and src[k] != q:
                        k += 2 if src[k] == "\\" else 1
                elif ch == "`":
                    k, _p, _n = _read_template(src, k)
                    nl += _n
                    continue
                elif ch == "\n":
                    nl += 1
                k += 1
            parts.append(("e", src[j + 2 : k - 1]))
            j = k
            continue
        if c == "\n":
            nl += 1
        buf.append(c)
        j += 1
    return n, parts, nl


def lex(src: str) -> list[Tok]:
    toks: list[Tok] = []
    i, n, line = 0, len(src), 1
    while i < n:
        c = src[i]
        if c == "\n":
            line += 1
            i += 1
            continue
        if c in " \t\r":
            i += 1
            continue
        if src.startswith("//", i):
            while i < n and src[i] != "\n":
                i += 1
            continue
        if src.startswith("/*", i):
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            line += src.count("\n", i, j)
            i = j
            continue
        if c == "`":
            j, parts, nl = _read_template(src, i)
            toks.append(Tok("tpl", src[i + 1 : j - 1], i, j, line, parts))
            line += nl
            i = j
            continue
        if c in "'\"":
            j = i + 1
            buf = []
            ok = False
            while j < n and src[j] != "\n":
                if src[j] == "\\" and j + 1 < n:
                    buf.append(src[j + 1])
                    j += 2
                    continue
                if src[j] == c:
                    ok = True
                    break
                buf.append(src[j])
                j += 1
            if ok:
                toks.append(Tok("str", "".join(buf), i, j + 1, line))
                i = j + 1
            else:  # stray quote (JSX text apostrophe etc.)
                toks.append(Tok("punct", c, i, i + 1, line))
                i += 1
            continue
        if c in ID_START:
            j = i + 1
            while j < n and src[j] in ID_PART:
                j += 1
            toks.append(Tok("id", src[i:j], i, j, line))
            i = j
            continue
        if c.isdigit():
            j = i + 1
            while j < n and (src[j].isalnum() or src[j] in "._"):
                j += 1
            toks.append(Tok("num", src[i:j], i, j, line))
            i = j
            continue
        if c == "/":
            prev = toks[-1] if toks else None
            is_regex = prev is None or (
                (prev.kind == "punct" and prev.text in REGEX_PREV_PUNCT)
                or (prev.kind == "id" and prev.text in REGEX_PREV_KW)
            )
            if is_regex:
                j, in_cls, ok = i + 1, False, False
                while j < n and src[j] != "\n":
                    ch = src[j]
                    if ch == "\\":
                        j += 2
                        continue
                    if ch == "[":
                        in_cls = True
                    elif ch == "]":
                        in_cls = False
                    elif ch == "/" and not in_cls:
                        ok = True
                        break
                    j += 1
                if ok:
                    j += 1
                    while j < n and src[j].isalpha():
                        j += 1
                    toks.append(Tok("regex", src[i:j], i, j, line))
                    i = j
                    continue
        for m in MULTI:
            if src.startswith(m, i):
                toks.append(Tok("punct", m, i, i + len(m), line))
                i += len(m)
                break
        else:
            toks.append(Tok("punct", c, i, i + 1, line))
            i += 1
    return toks


CLOSERS = {"(": ")", "[": "]", "{": "}"}


def match_close(toks: list[Tok], i: int) -> int:
    """toks[i] is an opener ( [ {. Return index of the matching closer (or len(toks)-1)."""
    stack = []
    for j in range(i, len(toks)):
        t = toks[j]
        if t.kind != "punct":
            continue
        if t.text in CLOSERS:
            stack.append(CLOSERS[t.text])
        elif t.text in (")", "]", "}"):
            if stack and stack[-1] == t.text:
                stack.pop()
                if not stack:
                    return j
            else:  # mismatched: bail out gracefully
                return j
    return len(toks) - 1


def split_top(toks: list[Tok], lo: int, hi: int, sep: str = ",") -> list[tuple[int, int]]:
    """Split toks[lo:hi] at top-level `sep`. Returns list of (start, end) index pairs (end exclusive)."""
    out, depth, start = [], 0, lo
    angle = 0
    for j in range(lo, hi):
        t = toks[j]
        if t.kind == "punct":
            if t.text in "([{":
                depth += 1
            elif t.text in ")]}":
                depth -= 1
            elif t.text == sep and depth == 0 and angle == 0:
                out.append((start, j))
                start = j + 1
    if start < hi:
        out.append((start, hi))
    return out


def src_of(src: str, toks: list[Tok], lo: int, hi: int) -> str:
    if lo >= hi:
        return ""
    return src[toks[lo].pos : toks[hi - 1].end]
