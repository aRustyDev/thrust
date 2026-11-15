"""Sizes of one worker-build output: bytes, gzip -9 bytes and sha256 of index_bg.wasm, index.js and worker/shim.mjs.

Usage: sizes.py <label> <build-dir> [needle ...]
Each needle (a text string) is searched for in index_bg.wasm and its count is printed: the positive control of step A4.
Same method as spike 0001 (research/0004 section 6.1): wc -c and gzip -9 -c on the uncompressed files.
"""
import gzip
import hashlib
import os
import sys

label, build = sys.argv[1], sys.argv[2]
needles = []
for a in sys.argv[3:]:
    if a.startswith("@"):  # @file: one needle per line
        with open(a[1:], encoding="utf-8") as nf:
            needles += [l.rstrip("\n") for l in nf if l.strip()]
    else:
        needles.append(a)
files = ["index_bg.wasm", "index.js", "worker/shim.mjs"]
total = 0
total_gz = 0
for f in files:
    with open(os.path.join(build, f), "rb") as fh:
        data = fh.read()
    gz = len(gzip.compress(data, compresslevel=9, mtime=0))
    total += len(data)
    total_gz += gz
    print(f"{label}\t{f}\tbytes={len(data)}\tgzip9={gz}\tsha256={hashlib.sha256(data).hexdigest()[:16]}")
print(f"{label}\tTOTAL\tbytes={total}\tMiB={total / 1048576:.3f}\tgzip9={total_gz}")
with open(os.path.join(build, "index_bg.wasm"), "rb") as fh:
    wasm = fh.read()
for n in needles:
    print(f"{label}\tneedle {n!r} in index_bg.wasm: {wasm.count(n.encode())}")
