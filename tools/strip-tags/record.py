"""Record what Django's `strip_tags` answers, as the fixture of `thrust-html`'s `parity_strip_tags_*` test (issue #48).

`thrust_html::strip_tags` is a port of `django.utils.html.strip_tags`, the function behind a work item's `description_stripped`. The
port's test compares it with what the real function answered, recorded here and never edited by hand (AGENTS.md testing rule 9 (c)).

The oracle is `django_strip_tags` of tools/spike-0006/sanitize/run.py: Django's own loop and `MLStripper` over the standard library's
`html.parser.HTMLParser`, which is all `strip_tags` is made of (no Django install is needed). Its answer depends on the Python's
parser, so the fixture says which Python recorded it; Plane's image runs 3.12.

The inputs are what `strip_tags` is given in production: a description is cleaned before its text is taken, so they are the default
sanitiser's output for every entry of the committed XSS corpus and every document of the editor corpus (a Rust example writes them), and
the hand-written cases below, which are not cleaned and include what the sanitiser never lets through (a comment or a script that is
not closed): the port does not match Python on two of those, and the test names them.

    scripts/rs cargo run -p thrust-html --example clean_corpus -- /tmp/cleaned.json
    uv run --python 3.12 --no-project python -I tools/strip-tags/record.py /tmp/cleaned.json

writes crates/core/html/tests/parity/strip_tags.json.
"""

import importlib.util
import json
import platform
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "crates/core/html/tests/parity/strip_tags.json"

spec = importlib.util.spec_from_file_location("sanitize_run", ROOT / "tools/spike-0006/sanitize/run.py")
run = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run)
strip_tags = run.django_strip_tags

# What the corpora do not reach: entity and character references in every form, text that only looks like a tag, quotes and `>` inside
# attributes, comments, declarations and processing instructions (closed and not), raw-text elements, and input whose stripping makes a
# new tag (which is why Django loops).
EDGE = [
    "",
    "plain text, no markup at all",
    "AT&T and Q&A",
    "&amp; &lt; &gt; &quot; &nbsp; &copy;",
    "&amp",
    "&nbsp",
    "&#65 &#x41 &#X41;",
    "&#65;",
    "&#xZZ;",
    "&1 &-x &a.b",
    "a&b;c",
    "x &unknownentity; y",
    "&#;",
    "&;",
    "a < b and c > d",
    "1<2",
    "<",
    ">",
    "<>",
    "< p>",
    "<3 hearts",
    "a <b",
    "<p>one</p><p>two</p>",
    '<p class="a" id=\'b\'>x</p>',
    '<p title="a > b">x</p>',
    "<p title='a > b'>x</p>",
    '<p title="unterminated>x</p>',
    "<img src=x onerror=alert(1)>y",
    "<br/>z<br />",
    '<a href="x">l</a>',
    "</p>",
    "</>",
    "<//p>",
    "<p",
    "<p ",
    "<a href='x",
    "text <b>bold</b> text",
    "<b><i>nested</i></b>",
    "<!-- c -->x",
    "x<!-- c -->y<!-- d -->z",
    "<!-- unterminated",
    "<!---->x",
    "<!-->x",
    "<!DOCTYPE html>z",
    "<!doctype html><p>q</p>",
    "<![CDATA[x]]>y",
    "<![if IE]>z",
    "<?php echo 1 ?>q",
    '<?xml version="1.0"?>r',
    "<script>alert(1)</script>t",
    "<style>p{color:red}</style>u",
    "<script>if (a<b) x()</script>v",
    "<<b>script>alert(1)<</b>/script>",
    "<<p>p>x</p>",
    "<scr<b>ipt>x",
    "a<<b>b>c",
    "<p>日本語 — it's</p>",
    "<p>\n  line one\n  line two\n</p>",
    "<p>\t</p>",
    "<p>1 <b>2</b> &amp; 3 &lt;4&gt;</p>",
    "<ul><li>a</li><li>b &amp; c</li></ul>",
    # an entity or character reference is only completed (`&amp` to `&amp;`) when the text has a tag to strip, because Django's loop
    # does not run without one: so these have both (found by mutation testing: nothing above completed an entity)
    "<p>&amp</p>",
    "<p>AT&T and Q&A</p>",
    "<p>&#65 and &#x41 and &#X41;</p>",
    "<p>&nbsp text</p>",
    "<b>&a.b-c &1 &-x</b>",
    "<p>&#xZZ; &#; &;</p>",
    "<p>x&unknownentity;y &copy</p>",
    "<p>&#65</p><p>&#x41</p>",
    # a malformed character reference at the very end of the text, with no `;` after it: Python's parser and the port agree there (a
    # malformed one with more text after it is `edge:66`, where they do not). Found by mutation testing: the branch that reads a
    # reference's digits was only reached by well-formed ones and by `edge:66`.
    "<p>x</p>&#",
    "<p>x</p>&#Z",
    "<p>x</p>&#x",
    "<p>x</p>&#xZ",
    "<p>x</p>&#65",
    "<p>x</p>&#x41",
]


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: record.py <the cleaned corpora: the output of the clean_corpus example>")
    cases = []
    for c in json.loads(Path(sys.argv[1]).read_text()):
        cases.append({"from": c["from"], "in": c["in"], "out": strip_tags(c["in"])})
    for i, text in enumerate(EDGE):
        cases.append({"from": f"edge:{i}", "in": text, "out": strip_tags(text)})

    ids = [c["from"] for c in cases]
    assert len(ids) == len(set(ids)), "an id is used twice"
    header = {
        "oracle": "tools/spike-0006/sanitize/run.py django_strip_tags: Django's strip_tags loop and MLStripper over html.parser.HTMLParser",
        "recorded_by": "tools/strip-tags/record.py",
        "python": platform.python_version(),
    }
    lines = [json.dumps(c, ensure_ascii=False) for c in cases]
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(
        '{\n"oracle": ' + json.dumps(header["oracle"]) + ',\n"recorded_by": ' + json.dumps(header["recorded_by"]) + ',\n"python": '
        + json.dumps(header["python"]) + ',\n"cases": [\n' + ",\n".join(lines) + "\n]\n}\n"
    )
    print(f"{len(cases)} cases (Python {header['python']}) -> {OUT.relative_to(ROOT)}", file=sys.stderr)


main()
