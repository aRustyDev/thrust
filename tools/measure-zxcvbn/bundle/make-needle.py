"""Print needles for the A4 control: a long contiguous slice of zxcvbn 3.1.1's PASSWORDS list and of its ENGLISH_WIKI list.

Reads the crate from stdin (curl -sL https://static.crates.io/crates/zxcvbn/zxcvbn-3.1.1.crate | python3 -I make-needle.py).
A slice is cut between commas, from the middle of the list, and is restricted to [a-z0-9,] so it passes through a shell untouched.
It is a run of ~12 entries, so it cannot occur by accident in another crate's data.
"""
import io
import re
import sys
import tarfile

t = tarfile.open(fileobj=io.BytesIO(sys.stdin.buffer.read()), mode="r:gz")
src = t.extractfile("zxcvbn-3.1.1/src/frequency_lists.rs").read().decode()
consts = {m.group(1): m.group(2) for m in re.finditer(r'const ([A-Z_]+): &str = "([^"]*)";', src)}
for name in ("PASSWORDS", "ENGLISH_WIKI"):
    items = consts[name].split(",")
    start = len(items) // 2
    run = []
    i = start
    while len(run) < 12 and i < len(items):
        if re.fullmatch(r"[a-z0-9]+", items[i]):
            run.append(items[i])
        else:
            run = []
        i += 1
    print(",".join(run))
