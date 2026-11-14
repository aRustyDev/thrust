#!/usr/bin/env python3
"""Walk `bd --help` recursively and write the command tree. Read-only: it only runs `bd <cmd> --help`.

usage: uv run walk_bd.py <outdir>

Writes <outdir>/tree.json and <outdir>/help/<path>.txt (the raw help of every command). tree.json maps
"cmd sub" to {short, aliases, flags, children, desc, group}; the root key is "bd" and also carries
`version`. `group` is bd's own top-level heading for the command. Feed tree.json to classify.py.

Two cobra quirks this parser has to respect. Both bit the first pass of research/0015:

- cobra pads names to the longest one, so the longest command in a listing has ONE space before its
  description. A parser that wants two spaces silently drops it (the first pass lost 7 commands that way,
  among them `epic close-eligible`, which writes).
- Below the root only "Available Commands:" is a command list. A long description can hold indented
  `name  - text` lines (`bd gate` has a "Gate types:" block) that look like subcommands, and an unknown
  subcommand falls through to its parent's help, so reading them as commands recurses forever.
"""
import json
import os
import re
import subprocess
import sys

OUT = ""  # set by main()

MAX_DEPTH = 4
HEADER = re.compile(r"^([A-Z][A-Za-z &/\-]*):\s*$")
CMD_LINE = re.compile(r"^  (\S+)\s+(\S.*)$")  # one space is enough: see the docstring
REAL_CMD_SECTIONS = {"available commands", "additional commands"}


def run(argv):
    p = subprocess.run(["bd", *argv], capture_output=True, text=True, timeout=60)
    return p.stdout


def parse(text, root):
    lines = text.splitlines()
    short = lines[0].strip() if lines else ""
    children, flags, aliases = [], [], []
    section = None
    for ln in lines:
        m = HEADER.match(ln)
        if m:
            section = m.group(1)
            continue
        if not ln.strip():
            section = None  # a blank line ends any section
            continue
        low = (section or "").lower()
        if low == "aliases":
            aliases += [a.strip() for a in ln.split(",")]
        elif low == "flags":
            fm = re.search(r"(--[a-z0-9\-]+)", ln)
            if fm:
                flags.append(fm.group(1))
        elif section and (root or low in REAL_CMD_SECTIONS) and low not in {"global flags", "usage", "examples"}:
            # at the root bd defines its own group headings; below it only cobra's headers count
            cm = CMD_LINE.match(ln)
            if cm:
                children.append((cm.group(1), cm.group(2).strip(), section))
    return short, children, flags, aliases


tree = {}


def walk(argv):
    if len(argv) > MAX_DEPTH:
        raise SystemExit("depth cap hit at: " + " ".join(argv))
    text = run([*argv, "--help"])
    key = " ".join(argv) if argv else "bd"
    with open(os.path.join(OUT, "help", ("_".join(argv) if argv else "bd") + ".txt"), "w") as fh:
        fh.write(text)
    short, children, flags, aliases = parse(text, root=not argv)
    tree[key] = {
        "short": short,
        "aliases": aliases,
        "flags": flags,
        "children": [c[0] for c in children if c[0] not in ("help", "completion")],
    }
    for name, desc, group in children:
        if name == "help":
            continue
        child = argv + [name]
        walk(child)
        tree[" ".join(child)]["desc"] = desc
        if not argv:
            tree[name]["group"] = group


def main():
    global OUT
    OUT = sys.argv[1]
    os.makedirs(os.path.join(OUT, "help"), exist_ok=True)
    walk([])
    tree["bd"]["version"] = run(["version"]).strip().splitlines()[0]
    with open(os.path.join(OUT, "tree.json"), "w") as fh:
        json.dump(tree, fh, indent=1)
    print(f"{len(tree) - 1} command paths walked, root excluded ({tree['bd']['version']})")


if __name__ == "__main__":
    main()
