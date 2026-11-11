"""Print `crates/thrust-api-v1/tests/parity/fixtures/slugify.json`: what the pinned Plane's `slugify` does to a name.

A state's `slug` is `slugify(self.name)` in `State.save()` (`apps/api/plane/db/models/state.py:118`, tag
`oracle/v1-pin-20261018`), with `slugify` from `django.template.defaultfilters`: a filter that calls
`django.utils.text.slugify`, which is `unicodedata.normalize("NFKD")`, then ASCII only, then lower case, then what is not a word
character, a space or a hyphen dropped, runs of hyphens and spaces made one hyphen, and `-` and `_` stripped from both ends.

Django 5.2.17 and Python 3.12 are the pin's (`apps/api/uv.lock`, `apps/api/.python-version`); the Unicode data is Python's
`unicodedata` (15.0.0 for 3.12), which is what the pin folds with. The script refuses to run on any other pair, so that a fixture
that says it is the pin's is the pin's:

    uv run --no-project --python 3.12 --with django==5.2.17 python -I tools/compat-record/gen-slugify.py \
        > crates/thrust-api-v1/tests/parity/fixtures/slugify.json

The fixture holds three things, none picked by hand except the pool the names are drawn from:
  * `alone`: every Unicode scalar value (surrogates are not characters, and a Rust `str` cannot hold one) whose slug, as a name
    of that one character, is not empty. `é` is `e`, `²` is `2`, `ﬃ` is `ffi`, `日` is empty (so is not listed).
  * `between`: every scalar value `c` whose slug as `"a" + c + "b"` is not `"ab"`: what a character does between two letters, which
    is where a folded space (U+00A0 and the other Unicode spaces fold to U+0020) and a folded hyphen (U+FF0D) show.
  * `unassigned`: the ranges of scalar values that the pin's Unicode data has no character for (category `Cn`). The pin cannot fold
    what its Python does not know, and a Rust crate on newer Unicode data folds some of them (U+209D, a subscript letter, is one);
    the test lets a difference stand only inside these ranges, and fails any other.
  * `names`: names of up to twelve characters drawn, from a fixed seed, out of a pool of ASCII, spaces, hyphens, underscores, accents,
    compatibility characters, characters nothing folds (`ß`, `Ø`, CJK, emoji) and combining marks, with the slug of each.

Regenerate it only when the pin moves.
"""

import json
import random
import sys
import unicodedata

import django
from django.template.defaultfilters import slugify

PIN_DJANGO = "5.2.17"
PIN_PYTHON = (3, 12)
SEED = 58  # thrust issue #58
NAMES = 3000

if django.get_version() != PIN_DJANGO or sys.version_info[:2] != PIN_PYTHON:
    sys.exit(f"gen-slugify: the pin runs Django {PIN_DJANGO} on Python {PIN_PYTHON}, this is "
             f"Django {django.get_version()} on Python {sys.version_info[:2]}")


def scalars():
    return [c for c in map(chr, range(0x110000)) if not 0xD800 <= ord(c) <= 0xDFFF]


def emit_map(name, mapping, last=False):
    print(f'  "{name}": {{')
    items = list(mapping.items())
    for i, (k, v) in enumerate(items):
        print(f'    {json.dumps(k)}: {json.dumps(v)}{"," if i + 1 < len(items) else ""}')
    print("  }" + ("" if last else ","))


every = scalars()
alone = {f"{ord(c):04x}": s for c in every if (s := slugify(c))}
between = {f"{ord(c):04x}": s for c in every if (s := slugify("a" + c + "b")) != "ab"}

unassigned = []  # [first, last] of each run of consecutive scalar values with no character in the pin's Unicode data
for c in every:
    if unicodedata.category(c) != "Cn":
        continue
    if unassigned and unassigned[-1][1] == ord(c) - 1:
        unassigned[-1][1] = ord(c)
    else:
        unassigned.append([ord(c), ord(c)])

POOL = (
    list("aAbBzZ09") + ["_", "-", "--", "_-", " ", "\t", "\n", "\r", "\x0b", "\x0c", "\x1c", "\x1f", "!", ".", "(", "'", "%"]
    # what NFKD folds to ASCII: accents, compatibility forms, spaces and a hyphen
    + ["é", "É", "ñ", "ü", "ç", "²", "³", "ﬁ", "ﬃ", "Ａ", "ａ", "①",
       "Ⅷ", "㎏", " ", " ", "　", " ", "\u0085", "－", "K", "İ", "Å",
       "é", "́"]
    # what nothing folds: other letters, CJK, emoji, format characters, the hyphen and the soft hyphen
    + ["ß", "Ø", "æ", "đ", "ł", "日", "本", "\U0001f600", "​", "‍", "‐", "­"]
)
rng = random.Random(SEED)
names = []
for _ in range(NAMES):
    name = "".join(rng.choice(POOL) for _ in range(rng.randint(0, 12)))
    names.append([name, slugify(name)])

print("{")
print(f'  "source": "django.template.defaultfilters.slugify, Django {django.get_version()}, Python '
      f'{sys.version.split()[0]}, unicodedata {unicodedata.unidata_version}: the pin\'s (tag oracle/v1-pin-20261018); '
      f'tools/compat-record/gen-slugify.py",')
print(f'  "unicode_version": {json.dumps(unicodedata.unidata_version)},')
print(f'  "scalar_values": {len(every)},')
print(f'  "seed": {SEED},')
emit_map("alone", alone)
emit_map("between", between)
print('  "unassigned": [')
for i, (first, last) in enumerate(unassigned):
    print(f'    ["{first:04x}", "{last:04x}"]{"," if i + 1 < len(unassigned) else ""}')
print("  ],")
print('  "names": [')
for i, (name, slug) in enumerate(names):
    print(f'    [{json.dumps(name)}, {json.dumps(slug)}]{"," if i + 1 < len(names) else ""}')
print("  ]")
print("}")
