"""Spike 0006 sanitiser harness.

  uv run run.py corpus  --out xss-corpus.json
  uv run run.py oracle  --plane-tree <pinned Plane tree> --corpus xss-corpus.json --html corpus.json --out xss-oracle.json
  uv run run.py check   --corpus xss-corpus.json --oracle xss-oracle.json --rust xss-rust.json [--rust-old xss-rust-400.json]

`oracle` runs Plane's REAL `validate_html_content` (apps/api/plane/utils/content_validator.py, loaded from the pinned
tree, not re-typed) with the pinned nh3 0.2.18, both as the app serializers call it and as the /api/v1 IssueSerializer
calls it (lxml parse + re-serialise first).
`check` is the independent XSS checker (lxml + html5lib, with a second parse after serialising to catch mutation),
the must-fail controls, and the nh3 vs ammonia differential.
"""

from __future__ import annotations

import argparse
import html as pyhtml
import importlib.util
import json
import re
import sys
import types
from pathlib import Path


# --------------------------------------------------------------------------------------------- corpus
def cmd_corpus(a):
    from xss_corpus import CORPUS

    Path(a.out).write_text(json.dumps(CORPUS, ensure_ascii=False, indent=0) + "\n")
    mal = sum(1 for p in CORPUS if p["malicious"])
    print(f"{len(CORPUS)} payloads ({mal} malicious, {len(CORPUS) - mal} inert/benign) -> {a.out}")


# --------------------------------------------------------------------------------------------- oracle
def load_validator(plane_tree: str):
    # stub only the logging helper; everything else in the module is Plane's own code
    pkg = types.ModuleType("plane")
    utils = types.ModuleType("plane.utils")
    exc = types.ModuleType("plane.utils.exception_logger")
    exc.log_exception = lambda e, *a, **k: None
    sys.modules.update({"plane": pkg, "plane.utils": utils, "plane.utils.exception_logger": exc})
    path = Path(plane_tree) / "apps/api/plane/utils/content_validator.py"
    spec = importlib.util.spec_from_file_location("content_validator", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def v1_pre_step(html: str) -> str:
    """IssueSerializer.validate in /api/v1: lxml parse + serialise before the sanitiser."""
    from lxml import html as lh

    parsed = lh.fromstring(html)
    return lh.tostring(parsed, encoding="unicode")


def django_strip_tags(value: str) -> str:
    """django.utils.html.strip_tags (Django 5.x), the function behind `description_stripped`: stdlib HTMLParser based."""
    from html.parser import HTMLParser

    class MLStripper(HTMLParser):
        def __init__(self):
            super().__init__(convert_charrefs=False)
            self.reset()
            self.fed = []

        def handle_data(self, d):
            self.fed.append(d)

        def handle_entityref(self, name):
            self.fed.append("&%s;" % name)

        def handle_charref(self, name):
            self.fed.append("&#%s;" % name)

        def get_data(self):
            return "".join(self.fed)

    def _strip_once(v):
        s = MLStripper()
        s.feed(v)
        s.close()
        return s.get_data()

    value = str(value)
    while "<" in value and ">" in value:
        new_value = _strip_once(value)
        if value.count("<") == new_value.count("<"):
            break
        value = new_value
    return value


def cmd_oracle(a):
    mod = load_validator(a.plane_tree)
    import nh3

    items = json.loads(Path(a.corpus).read_text())
    inputs = {p["id"]: p["html"] for p in items}
    if a.html:
        corpus = json.loads(Path(a.html).read_text())
        for d in corpus["docs"]:
            h = d.get("html_in") or d.get("html")
            if h:
                inputs[f"doc:{d['id']}"] = h
    out = {}
    for k, h in inputs.items():
        ok, msg, clean = mod.validate_html_content(h) if h else (True, None, None)
        rec = {"app": {"valid": ok, "clean": clean, "stripped": django_strip_tags(clean) if clean else None}}
        try:
            pre = v1_pre_step(h)
            ok1, _msg1, clean1 = mod.validate_html_content(pre)
            rec["v1"] = {"valid": ok1, "pre": pre, "clean": clean1}
        except Exception as e:  # noqa: BLE001 - mirrors `except Exception: raise ValidationError("Invalid HTML passed")`
            rec["v1"] = {"valid": False, "error": f"{type(e).__name__}: {e}"}
        out[k] = rec
    meta = {"nh3": nh3.__version__ if hasattr(nh3, "__version__") else "0.2.18", "validator": str(Path(a.plane_tree) / "apps/api/plane/utils/content_validator.py")}
    Path(a.out).write_text(json.dumps({"meta": meta, "results": out}, ensure_ascii=False) + "\n")
    print(f"{len(out)} inputs through validate_html_content -> {a.out}")


# --------------------------------------------------------------------------------------------- checker
BAD_ELEMENTS = {"script", "iframe", "frame", "frameset", "object", "embed", "applet", "base", "meta", "link", "form", "svg", "math", "style", "isindex", "template", "xmp", "plaintext"}
URL_ATTRS = {"href", "src", "action", "formaction", "data", "poster", "background", "xlink:href", "cite", "longdesc", "manifest", "ping", "usemap", "lowsrc", "dynsrc", "icon", "profile", "srcset", "entity_identifier_url", "data-emoji-url"}
BAD_SCHEMES = re.compile(r"^(javascript|vbscript|livescript|mocha|data):", re.I)
EVENT = re.compile(r"^on[a-z]+$", re.I)
CSS_BAD = re.compile(r"expression\(|javascript:|vbscript:|behavior\s*:|-moz-binding|@import|url\(\s*['\"]?\s*(javascript|vbscript|data):", re.I)


def _norm_url(v: str) -> str:
    return re.sub(r"[\x00-\x20\x7f-\x9f]", "", v)


def _walk_lxml(root, found, tag):
    for el in root.iter():
        if not isinstance(el.tag, str):
            continue
        name = el.tag.split("}")[-1].lower()
        if name in BAD_ELEMENTS:
            found.append(f"{tag}: <{name}> element")
        for k, v in el.attrib.items():
            lk = k.lower().split("}")[-1]
            if EVENT.match(lk):
                found.append(f"{tag}: event handler attribute {lk} on <{name}>")
            if lk == "srcdoc":
                found.append(f"{tag}: srcdoc attribute on <{name}>")
            if lk in URL_ATTRS and BAD_SCHEMES.match(_norm_url(v or "")):
                found.append(f"{tag}: dangerous URL scheme in {lk} on <{name}>")
            if lk == "style" and CSS_BAD.search(re.sub(r"/\*.*?\*/|\\", "", v or "")):
                found.append(f"{tag}: dangerous CSS in style on <{name}>")


def findings(s: str) -> list[str]:
    """Independent verdict on a sanitiser OUTPUT: parse with libxml2 and html5lib, and parse again after re-serialising."""
    import html5lib
    from lxml import html as lh

    found: list[str] = []
    if not s:
        return found
    try:  # whole-document parses keep <html>/<body> attributes that fragment parsing drops
        _walk_lxml(lh.document_fromstring(s), found, "lxml-doc")
        doc = html5lib.parse(s, treebuilder="lxml", namespaceHTMLElements=False)
        _walk_lxml(doc.getroot() if hasattr(doc, "getroot") else doc, found, "html5lib-doc")
    except Exception:  # noqa: BLE001 - empty/odd documents; the fragment parses below still run
        pass
    try:
        frag = lh.fragment_fromstring(s, create_parent="div")
        _walk_lxml(frag, found, "lxml")
        again = lh.tostring(frag, encoding="unicode")
        frag2 = lh.fragment_fromstring(again, create_parent="div")
        _walk_lxml(frag2, found, "lxml-reparsed")
    except Exception as e:  # noqa: BLE001 - an unparsable output is reported, not hidden
        found.append(f"lxml could not parse the output: {e}")
    try:
        doc = html5lib.parseFragment(s, treebuilder="lxml", namespaceHTMLElements=False)
        for el in doc:
            if hasattr(el, "iter"):
                _walk_lxml(el, found, "html5lib")
        ser = html5lib.serialize(html5lib.parseFragment(s, treebuilder="etree", namespaceHTMLElements=False), tree="etree", quote_attr_values="always", omit_optional_tags=False)
        doc2 = html5lib.parseFragment(ser, treebuilder="lxml", namespaceHTMLElements=False)
        for el in doc2:
            if hasattr(el, "iter"):
                _walk_lxml(el, found, "html5lib-reparsed")
    except Exception as e:  # noqa: BLE001
        found.append(f"html5lib could not parse the output: {e}")
    # de-duplicate, keep order
    seen, uniq = set(), []
    for f in found:
        if f not in seen:
            seen.add(f)
            uniq.append(f)
    return uniq


ADVISORY_ATTR = re.compile(r"\s(id|name)\s*=|position\s*:\s*fixed", re.I)


def naive_script_stripper(s: str) -> str:
    return re.sub(r"<script.*?>.*?</script>", "", s, flags=re.I | re.S)


def naive_handler_stripper(s: str) -> str:
    return re.sub(r"\son\w+\s*=\s*(\"[^\"]*\"|'[^']*'|[^\s>]+)", "", naive_script_stripper(s), flags=re.I)


def cmd_check(a):
    corpus = json.loads(Path(a.corpus).read_text())
    oracle = json.loads(Path(a.oracle).read_text())["results"]
    rust = json.loads(Path(a.rust).read_text())
    rust_old = json.loads(Path(a.rust_old).read_text()) if a.rust_old else None
    report: dict = {"payloads": len(corpus), "malicious": sum(1 for p in corpus if p["malicious"])}

    # ---- control 1: the checker must flag every malicious payload when it is NOT sanitised
    unsanitised_flag = {p["id"]: findings(p["html"]) for p in corpus}
    missed = [p["id"] for p in corpus if p["malicious"] and not unsanitised_flag[p["id"]]]
    fp = [p["id"] for p in corpus if not p["malicious"] and unsanitised_flag[p["id"]]]
    report["control_unsanitised"] = {"malicious_flagged": report["malicious"] - len(missed), "malicious_total": report["malicious"], "missed": missed, "inert_flagged_false_positives": fp}

    # ---- control 2: deliberately weak sanitisers must be caught by the same check
    def score(fn):
        bad = [p["id"] for p in corpus if p["malicious"] and findings(fn(p["html"]))]
        return {"unsafe_after": len(bad), "of": report["malicious"]}

    report["control_weak_sanitisers"] = {"identity": score(lambda s: s), "regex <script> stripper": score(naive_script_stripper), "regex script+on*= stripper": score(naive_handler_stripper)}

    # ---- the real contenders
    def verdicts(get):
        res = {}
        for p in corpus:
            out = get(p["id"])
            res[p["id"]] = None if out is None else findings(out)
        return res

    contenders = {
        "nh3 0.2.18 (Plane today, app path: validate_html_content)": lambda i: oracle[i]["app"]["clean"] if oracle[i]["app"]["valid"] else "",
        "nh3 0.2.18 (Plane today, /api/v1 path: lxml then validate_html_content)": lambda i: (oracle[i]["v1"]["clean"] if oracle[i]["v1"].get("valid") else ""),
        "Rust ammonia 4.2.4 (latest), Plane allow-list": lambda i: rust[i]["clean"],
    }
    if rust_old:
        contenders["Rust ammonia 4.0.0 (the version inside nh3 0.2.18), Plane allow-list"] = lambda i: rust_old[i]["clean"]
    contenders["Rust ammonia 4.2.4 + value filters on style/background/data-emoji-url (hardened, NOT nh3 parity)"] = lambda i: rust[i]["hardened"]
    report["contenders"] = {}
    for name, get in contenders.items():
        v = verdicts(get)
        mal_fail = [i for i, f in v.items() if f and next(p for p in corpus if p["id"] == i)["malicious"]]
        any_fail = [i for i, f in v.items() if f]
        report["contenders"][name] = {
            "malicious_payloads_neutralised": report["malicious"] - len(mal_fail),
            "malicious_total": report["malicious"],
            "failures": {i: v[i][:3] for i in mal_fail},
            "unexpected_findings_on_inert_payloads": {i: v[i][:3] for i in any_fail if i not in mal_fail},
        }

    # ---- differential: Rust ammonia vs nh3, byte for byte
    def diff(rust_map):
        d = [i for i in rust_map if i in oracle and oracle[i]["app"]["clean"] is not None and rust_map[i]["clean"] != oracle[i]["app"]["clean"]]
        n = [i for i in rust_map if i in oracle and oracle[i]["app"]["clean"] is not None]
        return {"identical": len(n) - len(d), "compared": len(n), "different_ids": d[:40]}

    report["differential_vs_nh3"] = {"ammonia 4.2.4": diff(rust)}
    if rust_old:
        report["differential_vs_nh3"]["ammonia 4.0.0"] = diff(rust_old)
        # strip_tags parity is only meaningful where both sides stripped the SAME cleaned html (ammonia 4.0.0 == nh3)
        same_clean = [i for i in rust_old if i in oracle and oracle[i]["app"]["clean"] and rust_old[i]["clean"] == oracle[i]["app"]["clean"]]
        bad = [i for i in same_clean if rust_old[i]["stripped"] != oracle[i]["app"]["stripped"]]
        report["strip_tags_vs_django"] = {"identical": len(same_clean) - len(bad), "compared": len(same_clean), "different_ids": bad[:20]}
        report["hardened_vs_plain_on_editor_html"] = {
            "docs": sum(1 for i in rust if i.startswith("doc:")),
            "output_identical": sum(1 for i in rust if i.startswith("doc:") and rust[i]["hardened"] == rust[i]["clean"]),
        }

    # ---- advisory: things Plane's allow-list keeps that are not script execution but are worth knowing
    adv = {}
    for p in corpus:
        c = oracle[p["id"]]["app"]["clean"]
        if c and ADVISORY_ATTR.search(c):
            adv[p["id"]] = [m.group(0).strip() for m in ADVISORY_ATTR.finditer(c)][:4]
    report["advisory_kept_by_allow_list"] = {"count": len(adv), "examples": dict(list(adv.items())[:8])}
    print(json.dumps(report, indent=1, ensure_ascii=False))


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("corpus")
    s.add_argument("--out", required=True)
    s.set_defaults(fn=cmd_corpus)
    s = sub.add_parser("oracle")
    s.add_argument("--plane-tree", required=True)
    s.add_argument("--corpus", required=True)
    s.add_argument("--html")
    s.add_argument("--out", required=True)
    s.set_defaults(fn=cmd_oracle)
    s = sub.add_parser("check")
    s.add_argument("--corpus", required=True)
    s.add_argument("--oracle", required=True)
    s.add_argument("--rust", required=True)
    s.add_argument("--rust-old")
    s.set_defaults(fn=cmd_check)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
