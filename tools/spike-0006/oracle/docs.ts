// Spike 0006 corpus: synthetic documents authored as ProseMirror JSON. No real Plane content.
// Every document is validated against the real schema before it is accepted (gen-corpus.ts).
import type { JSONContent as J } from "@tiptap/core";

export type AuthoredDoc = { id: string; desc: string; doc: J; title?: string };

// ---- tiny builders -------------------------------------------------------------------------
const t = (text: string, ...marks: (string | J)[]): J => ({
  type: "text",
  text,
  ...(marks.length ? { marks: marks.map((m) => (typeof m === "string" ? { type: m } : m)) } : {}),
});
const link = (href: string, extra: Record<string, unknown> = {}): J => ({ type: "link", attrs: { href, ...extra } });
const color = (attrs: Record<string, unknown>): J => ({ type: "customColor", attrs });
const p = (...c: (J | string)[]): J => ({ type: "paragraph", content: c.map((x) => (typeof x === "string" ? t(x) : x)) });
const pa = (textAlign: string, ...c: (J | string)[]): J => ({ ...p(...c), attrs: { textAlign } });
const ep = (): J => ({ type: "paragraph" });
const h = (level: number, ...c: (J | string)[]): J => ({
  type: "heading",
  attrs: { level },
  content: c.map((x) => (typeof x === "string" ? t(x) : x)),
});
const ul = (...items: J[]): J => ({ type: "bulletList", content: items });
const ol = (start: number | null, ...items: J[]): J => ({
  type: "orderedList",
  attrs: start === null ? {} : { start },
  content: items,
});
const li = (...c: (J | string)[]): J => ({
  type: "listItem",
  content: c.map((x) => (typeof x === "string" ? p(x) : x)),
});
const tl = (...items: J[]): J => ({ type: "taskList", content: items });
const ti = (checked: boolean, ...c: (J | string)[]): J => ({
  type: "taskItem",
  attrs: { checked },
  content: c.map((x) => (typeof x === "string" ? p(x) : x)),
});
const bq = (...c: J[]): J => ({ type: "blockquote", content: c });
const code = (language: string | null, text: string): J => ({
  type: "codeBlock",
  attrs: language === null ? {} : { language },
  ...(text ? { content: [{ type: "text", text }] } : {}),
});
const hr = (): J => ({ type: "horizontalRule" });
const br = (): J => ({ type: "hardBreak" });
const img = (attrs: Record<string, unknown>): J => ({ type: "image", attrs });
const imgc = (attrs: Record<string, unknown>): J => ({ type: "imageComponent", attrs });
const mention = (id: string, entity: string, name = "user_mention"): J => ({
  type: "mention",
  attrs: { id, entity_identifier: entity, entity_name: name },
});
const emoji = (name: string): J => ({ type: "emoji", attrs: { name } });
const callout = (attrs: Record<string, unknown>, ...c: J[]): J => ({ type: "calloutComponent", attrs, content: c });
const embed = (attrs: Record<string, unknown>): J => ({ type: "issue-embed-component", attrs });
const td = (c: (J | string)[], attrs: Record<string, unknown> = {}): J => ({
  type: "tableCell",
  attrs,
  content: c.map((x) => (typeof x === "string" ? p(x) : x)),
});
const th = (c: (J | string)[], attrs: Record<string, unknown> = {}): J => ({
  type: "tableHeader",
  attrs,
  content: c.map((x) => (typeof x === "string" ? p(x) : x)),
});
const tr = (...cells: J[]): J => ({ type: "tableRow", content: cells });
const tbl = (...rows: J[]): J => ({ type: "table", content: rows });
const doc = (...b: J[]): J => ({ type: "doc", content: b });

// stable synthetic ids (uuid-shaped, obviously fake)
const uid = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;

// ---- seeded PRNG for generated documents ---------------------------------------------------
function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let x = Math.imul(a ^ (a >>> 15), 1 | a);
    x = (x + Math.imul(x ^ (x >>> 7), 61 | x)) ^ x;
    return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
  };
}
const WORDS = (
  "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega " +
  "sprint backlog roadmap cycle module estimate triage release rollback deploy migrate schema index query cache queue worker " +
  "edge region latency budget quota policy review draft merge branch commit ticket owner reviewer"
).split(" ");
const MARKS = ["bold", "italic", "strike", "underline", "code"];

function sentence(r: () => number, n: number): string {
  const out: string[] = [];
  for (let i = 0; i < n; i++) out.push(WORDS[Math.floor(r() * WORDS.length)]);
  return out.join(" ");
}

function inlineRun(r: () => number, allowMention: boolean): J[] {
  const parts: J[] = [];
  const n = 1 + Math.floor(r() * 4);
  for (let i = 0; i < n; i++) {
    const roll = r();
    const text = sentence(r, 1 + Math.floor(r() * 5)) + (i < n - 1 ? " " : "");
    if (roll < 0.45) parts.push(t(text));
    else if (roll < 0.75) {
      const m1 = MARKS[Math.floor(r() * MARKS.length)];
      const marks = r() < 0.3 && m1 !== "code" ? [m1, "italic"] : [m1];
      parts.push(t(text, ...Array.from(new Set(marks))));
    } else if (roll < 0.85) parts.push(t(text, link(`https://example.com/${Math.floor(r() * 1000)}`)));
    else if (roll < 0.92) parts.push(t(text, color(r() < 0.5 ? { color: "green" } : { backgroundColor: "#ffcc00" })));
    else if (roll < 0.96 && allowMention) parts.push(mention(uid(1 + Math.floor(r() * 50)), uid(100 + Math.floor(r() * 50))));
    else parts.push(emoji(["smile", "rocket", "+1", "tada"][Math.floor(r() * 4)]));
  }
  // merge-safe: PM merges adjacent identical-mark text itself
  return parts;
}

function randomBlock(r: () => number, depth: number): J {
  const roll = r();
  if (roll < 0.3) return { type: "paragraph", content: inlineRun(r, true) };
  if (roll < 0.4) return { type: "heading", attrs: { level: 1 + Math.floor(r() * 6) }, content: inlineRun(r, false) };
  if (roll < 0.5 && depth < 3)
    return ul(...Array.from({ length: 1 + Math.floor(r() * 3) }, () => li(p(...inlineRun(r, true)), ...(r() < 0.3 ? [randomBlock(r, depth + 1)] : []))));
  if (roll < 0.58 && depth < 3)
    return ol(r() < 0.5 ? null : 1 + Math.floor(r() * 9), ...Array.from({ length: 1 + Math.floor(r() * 3) }, () => li(p(...inlineRun(r, true)))));
  if (roll < 0.64 && depth < 3)
    return tl(...Array.from({ length: 1 + Math.floor(r() * 3) }, () => ti(r() < 0.5, p(...inlineRun(r, true)))));
  if (roll < 0.7 && depth < 3) return bq(p(...inlineRun(r, true)), ...(r() < 0.3 ? [randomBlock(r, depth + 1)] : []));
  if (roll < 0.76) return code(["js", "ts", "python", null][Math.floor(r() * 4)] as any, sentence(r, 6) + "\n" + sentence(r, 4));
  if (roll < 0.8) return hr();
  if (roll < 0.86 && depth < 2)
    return tbl(
      tr(...Array.from({ length: 2 }, () => th([p(...inlineRun(r, false))]))),
      ...Array.from({ length: 1 + Math.floor(r() * 2) }, () => tr(...Array.from({ length: 2 }, () => td([p(...inlineRun(r, true))]))))
    );
  if (roll < 0.9 && depth < 2)
    return callout({ "data-background": "peach" }, p(...inlineRun(r, true)), ...(r() < 0.3 ? [randomBlock(r, depth + 1)] : []));
  if (roll < 0.95)
    return imgc({ id: uid(200 + Math.floor(r() * 90)), src: uid(300 + Math.floor(r() * 90)), status: "uploaded", width: "400px", height: "auto", aspectRatio: 1.5, alignment: "center" });
  return pa(["left", "center", "right"][Math.floor(r() * 3)], ...inlineRun(r, true));
}

function randomDoc(seed: number, blocks: number): J {
  const r = rng(seed);
  return doc(...Array.from({ length: blocks }, () => randomBlock(r, 0)));
}

// ---- the corpus ------------------------------------------------------------------------------
export function authoredDocs(): AuthoredDoc[] {
  const D: AuthoredDoc[] = [];
  const add = (id: string, desc: string, d: J, title?: string) => D.push({ id, desc, doc: d, ...(title !== undefined ? { title } : {}) });

  // paragraphs / text
  add("empty", "single empty paragraph (the empty document)", doc(ep()));
  add("para-plain", "one plain paragraph", doc(p("Hello Plane")));
  add("para-two", "two paragraphs", doc(p("First paragraph."), p("Second paragraph.")));
  for (const a of ["left", "center", "right"]) add(`para-align-${a}`, `paragraph textAlign=${a}`, doc(pa(a, `Aligned ${a}`)));
  add("para-unicode", "unicode: CJK, RTL, combining marks, astral plane, ZWJ emoji", doc(p("日本語のテキスト 漢字"), p("مرحبا بالعالم עברית"), p("é ñ ä"), p("𝔘𝔫𝔦𝔠𝔬𝔡𝔢 😀 👩‍👩‍👧‍👦 🏳️‍🌈")));
  add("para-specials", "HTML specials and quotes in text", doc(p(`a < b && c > d "quoted" 'single' &amp; literal`), p("</p><script>alert(1)</script>")));
  add("para-whitespace", "leading/trailing/multiple spaces, tab, nbsp", doc(p("  leading and trailing  "), p("a    b"), p("tab\there"), p("nbsp here  twice")));
  add("para-long-10k", "10 000 character paragraph", doc(p(Array.from({ length: 1000 }, (_, i) => `word${i}`).join(" ").slice(0, 10000))));
  add("para-long-200k", "200 000 character paragraph", doc(p(Array.from({ length: 20000 }, (_, i) => `token${i}`).join(" ").slice(0, 200000))));
  add("para-hardbreak", "hard breaks inside a paragraph, at the start and the end", doc({ type: "paragraph", content: [br(), t("line one"), br(), t("line two"), br()] }));
  for (let l = 1; l <= 6; l++) add(`heading-${l}`, `heading level ${l}`, doc(h(l, `Heading ${l}`)));
  add("heading-align", "heading textAlign center + marks", doc({ ...h(2, t("Centered ", "bold"), "heading"), attrs: { level: 2, textAlign: "center" } }));

  // marks
  for (const m of MARKS) add(`mark-${m}`, `mark ${m}`, doc(p("plain ", t("marked", m), " plain")));
  add("mark-link-http", "link https", doc(p("see ", t("the docs", link("https://example.com/docs?a=1&b=2#frag")))));
  add("mark-link-mailto", "link mailto/tel", doc(p(t("mail me", link("mailto:someone@example.com")), " or ", t("call", link("tel:+15551234")))));
  add("mark-link-relative", "relative and empty link", doc(p(t("rel", link("/workspace/projects")), " ", t("empty", link("")))));
  add("mark-link-custom-attrs", "link with custom target/rel/class", doc(p(t("custom", link("https://example.com", { target: "_self", rel: "noopener", class: "my-link" })))));
  add("mark-link-null-attrs", "link with null target/rel/class", doc(p(t("nulls", link("https://example.com", { target: null, rel: null, class: null })))));
  add("mark-textstyle", "textStyle mark (no attrs)", doc(p("x ", t("styled", "textStyle"), " y")));
  add("mark-color-named", "customColor with palette keys", doc(p(t("gray text", color({ color: "gray" })), " ", t("peach bg", color({ backgroundColor: "peach" })), " ", t("both", color({ color: "purple", backgroundColor: "green" })))));
  add("mark-color-hex", "customColor with hex values (rendered as style)", doc(p(t("hex fg", color({ color: "#ff0000" })), " ", t("hex bg", color({ backgroundColor: "#00ff00" })), " ", t("rgb", color({ color: "rgb(1, 2, 3)" })))));
  add("mark-adjacent", "adjacent marks of different kinds", doc(p(t("bold", "bold"), t("italic", "italic"), t("strike", "strike"), t("underline", "underline"), t("code", "code"))));
  add("mark-overlap", "partially overlapping marks", doc(p(t("a", "bold"), t("b", "bold", "italic"), t("c", "italic"), t("d", "italic", "underline"), t("e", "underline"))));
  add("mark-all-but-code", "every mark at once", doc(p(t("everything", "bold", "italic", "strike", "underline", link("https://example.com"), color({ color: "gray", backgroundColor: "peach" }), "textStyle"))));
  add("mark-link-bold-split", "link spanning differently marked runs", doc(p(t("one ", link("https://example.com")), t("two ", link("https://example.com"), "bold"), t("three", link("https://example.com"), "italic", "bold"))));
  add("mark-code-bold", "code mark excludes others", doc(p(t("code only", "code"), " and ", t("bold", "bold"))));
  add("mark-in-heading", "marks inside a heading", doc(h(3, t("Mixed ", "bold"), t("heading", "italic", "underline"))));
  add("mark-on-mention-neighbours", "marked text around mentions", doc(p(t("hi ", "bold"), mention(uid(1), uid(2)), t(" there", "italic"))));

  // lists
  add("list-bullet", "bullet list", doc(ul(li("one"), li("two"), li("three"))));
  add("list-ordered", "ordered list default start", doc(ol(null, li("one"), li("two"))));
  add("list-ordered-start", "ordered list start=5", doc(ol(5, li("five"), li("six"))));
  add("list-nested-3", "three levels of bullets", doc(ul(li(p("a"), ul(li(p("b"), ul(li("c"), li("d"))), li("e"))), li("f"))));
  add("list-nested-mixed", "bullets, ordered and tasks nested", doc(ul(li(p("bullet"), ol(null, li(p("ordered"), tl(ti(true, "done"), ti(false, "todo"))))))));
  add("list-nested-deep-12", "twelve nested bullet levels", (() => {
    let inner: J = ul(li("leaf"));
    for (let i = 0; i < 11; i++) inner = ul(li(p(`level ${11 - i}`), inner));
    return doc(inner);
  })());
  add("list-item-multi-block", "list item with several blocks", doc(ul(li(p("intro"), p("second paragraph"), code("js", "const x = 1;"), bq(p("quoted"))))));
  add("list-marks", "marks and links in list items", doc(ul(li(p(t("bold", "bold"), " and ", t("link", link("https://example.com")))), li(p(mention(uid(5), uid(6)), " assigned")))));
  add("task-list", "task list mixed", doc(tl(ti(true, "done"), ti(false, "todo"), ti(false, p("with ", t("marks", "bold"))))));
  add("task-nested", "nested task lists", doc(tl(ti(false, p("parent"), tl(ti(true, "child 1"), ti(false, "child 2", ), ti(false, p("child 3"), tl(ti(true, "grandchild"))))))));
  add("task-multi-block", "task item with extra blocks", doc(tl(ti(false, p("title"), p("details"), ul(li("sub"))))));

  // quotes, code, rules
  add("quote", "blockquote", doc(bq(p("quoted text"))));
  add("quote-nested", "nested blockquotes", doc(bq(p("outer"), bq(p("inner"), bq(p("innermost"))))));
  add("quote-with-blocks", "blockquote with list/code", doc(bq(p("q"), ul(li("item")), code("ts", "let y = 2;"))));
  add("quote-deep-40", "forty nested blockquotes", (() => {
    let inner: J = bq(p("core"));
    for (let i = 0; i < 39; i++) inner = bq(p(`depth ${39 - i}`), inner);
    return doc(inner);
  })());
  add("code-plain", "code block no language", doc(code(null, "plain text\nsecond line")));
  add("code-lang", "code block language js", doc(code("js", "function f(a, b) {\n  return a < b && b > 0;\n}\n")));
  add("code-empty", "empty code block", doc(code("python", "")));
  add("code-specials", "code with html specials and tabs", doc(code("html", "<div class=\"a\">&amp;\t'x'</div>")));
  add("code-long", "long code block", doc(code("rust", Array.from({ length: 300 }, (_, i) => `let v${i} = ${i};`).join("\n"))));
  add("code-then-para", "code block between paragraphs", doc(p("before"), code("sh", "echo hi"), p("after")));
  add("hr", "horizontal rule between paragraphs", doc(p("above"), hr(), p("below")));
  add("hr-consecutive", "consecutive rules", doc(hr(), hr(), hr()));

  // media / inline atoms
  add("img-external", "base image node, external URL", doc(img({ src: "https://example.com/a.png", alt: "alt text", title: "title" })));
  add("img-attrs", "base image node with size/alignment", doc(img({ src: "https://example.com/b.png", width: "50%", height: "200px", aspectRatio: 1.7777, alignment: "center" })));
  add("img-special-attrs", "image alt/title with quotes and angle brackets", doc(img({ src: "https://example.com/c.png?x=1&y=2", alt: "a \"quoted\" <alt> & more", title: "t'itle" })));
  add("imgc-basic", "imageComponent with asset id", doc(imgc({ id: uid(10), src: uid(11), status: "uploaded", width: "400px", height: "auto", aspectRatio: 1.5, alignment: "left" })));
  add("imgc-defaults", "imageComponent with only defaults", doc(imgc({})));
  add("imgc-aligns", "imageComponent alignments", doc(imgc({ src: uid(12), alignment: "center", status: "uploaded" }), imgc({ src: uid(13), alignment: "right", status: "uploaded" })));
  add("imgc-null-height", "imageComponent with null height (JS drops null on write)", doc(imgc({ src: uid(14), height: null, width: null, aspectRatio: null, status: "uploaded" })));
  add("imgc-pending", "imageComponent in pending/uploading states", doc(imgc({ id: uid(15), status: "pending" }), imgc({ id: uid(16), status: "uploading" }), imgc({ id: uid(17), status: "duplication-failed" })));
  add("mention-user", "user mention inside text", doc(p("ping ", mention(uid(20), uid(21)), " please")));
  add("mention-edges", "mentions at start, end and adjacent", doc(p(mention(uid(22), uid(23)), mention(uid(24), uid(25)), " text ", mention(uid(26), uid(27)))));
  add("mention-entities", "other mention entity kinds", doc(p(mention(uid(28), uid(29), "issue"), " ", mention(uid(30), uid(31), "page"), " ", mention(uid(32), uid(33), "project"), " ", mention(uid(34), uid(35), "cycle"), " ", mention(uid(36), uid(37), "module"))));
  add("mention-in-structures", "mentions in heading, list, table", doc(h(2, "Owner ", mention(uid(38), uid(39))), ul(li(p(mention(uid(40), uid(41))))), tbl(tr(td([p(mention(uid(42), uid(43)))])))));
  add("mention-nulls", "mention with missing attrs", doc(p({ type: "mention" })));
  add("emoji-basic", "emoji nodes", doc(p("deploy ", emoji("rocket"), emoji("tada"), " done ", emoji("+1"))));
  add("emoji-unknown", "emoji with an unknown name", doc(p("odd ", emoji("definitely_not_an_emoji"), " name")));
  add("emoji-null", "emoji with no name", doc(p({ type: "emoji" })));

  // tables
  add("table-2x2", "2x2 table with header row", doc(tbl(tr(th(["H1"]), th(["H2"])), tr(td(["a"]), td(["b"])))));
  add("table-3x3", "3x3 body-only table", doc(tbl(...[0, 1, 2].map((r) => tr(...[0, 1, 2].map((c) => td([`r${r}c${c}`])))))));
  add("table-span", "colspan/rowspan/colwidth", doc(tbl(tr(th(["wide"], { colspan: 2, colwidth: [100, 200] }), th(["h"])), tr(td(["tall"], { rowspan: 2 }), td(["x"]), td(["y"])), tr(td(["z"]), td(["w"])))));
  add("table-colors", "cell/row/header colours", doc(tbl(tr(th(["H"], { background: "#eeeeee" }), th(["I"], { background: "none" })), tr(td(["c"], { background: "peach", textColor: "gray" }), td(["d"], { background: "#00ff00", textColor: "#000000" })))));
  add("table-row-colors", "row-level background/textColor", doc(tbl({ ...tr(td(["a"]), td(["b"])), attrs: { background: "pink", textColor: "gray" } })));
  add("table-rich-cells", "cells with lists, code, images and marks", doc(tbl(tr(th(["Title"]), th(["Notes"])), tr(td([p(t("bold", "bold"))]), td([ul(li("a"), li("b")), code("js", "x()"), imgc({ src: uid(50), status: "uploaded" })])))));
  add("table-multi-para-cell", "cell with several paragraphs", doc(tbl(tr(td([p("one"), p("two"), p("three")])))));
  add("table-wide-10x10", "10x10 table", doc(tbl(tr(...Array.from({ length: 10 }, (_, c) => th([`C${c}`]))), ...Array.from({ length: 9 }, (_, r) => tr(...Array.from({ length: 10 }, (_, c) => td([`${r},${c}`])))))));
  add("table-hardbreak", "hard break in cell", doc(tbl(tr(td([{ type: "paragraph", content: [t("a"), br(), t("b")] }])))));

  // callouts / embeds
  add("callout-default", "callout with default emoji logo", doc(callout({}, p("Heads up"))));
  add("callout-icon", "callout with icon logo, colour and id", doc(callout({ id: uid(60), "data-logo-in-use": "icon", "data-icon-name": "Smile", "data-icon-color": "#ff0000", "data-background": "light-blue" }, p("icon callout"))));
  add("callout-blocks", "callout with list, code, quote", doc(callout({ "data-background": "green" }, p("intro"), ul(li("a"), li("b")), code("ts", "x"), bq(p("q")))));
  add("callout-nested", "callout inside callout", doc(callout({}, p("outer"), callout({ "data-background": "pink" }, p("inner")))));
  add("callout-table", "callout containing a table", doc(callout({}, tbl(tr(td(["in callout"]))))));
  add("embed-basic", "work-item embed (document schema only)", doc(p("see:"), embed({ id: uid(70), entity_identifier: uid(71), project_identifier: uid(72), workspace_identifier: uid(73), entity_name: "issue" })));
  add("embed-empty", "work-item embed with no attrs", doc(embed({})));

  // titles (page documents carry a second fragment)
  add("title-simple", "document with a page title fragment", doc(p("body")), "My page title");
  add("title-empty", "document with an empty page title", doc(p("body")), "");
  add("title-unicode", "document with a unicode title", doc(p("body"), ul(li("x"))), "Überschrift 日本語 😀");

  // realistic mixed documents
  add("real-bug-report", "bug-report style work item description", doc(
    h(2, "Summary"),
    p("Saving a page fails when the title contains ", t("emoji", "bold"), " and a ", t("link", link("https://example.com/issue/1")), "."),
    h(3, "Steps to reproduce"),
    ol(null, li("Open any page"), li("Type ", ) as any, li(p("Press ", t("Ctrl+S", "code")))),
    h(3, "Expected"),
    p("The page saves."),
    h(3, "Actual"),
    code("text", "Error: 413 content too large\n    at save (page.ts:42)"),
    imgc({ id: uid(80), src: uid(81), status: "uploaded", width: "600px", height: "auto", aspectRatio: 1.6, alignment: "center" }),
    tl(ti(true, "reproduced"), ti(false, "fixed"), ti(false, p("verified by ", mention(uid(82), uid(83))))),
    hr(),
    p(t("Filed by ", "italic"), mention(uid(84), uid(85)))
  ));
  add("real-spec-page", "page style document using most node types", doc(
    h(1, "Project plan"),
    callout({ "data-background": "light-blue" }, p("Status: ", t("draft", "bold"))),
    p("This plan covers ", t("three", "underline"), " milestones. See also ", mention(uid(90), uid(91), "page"), "."),
    tbl(tr(th(["Milestone"]), th(["Owner"]), th(["Due"])), tr(td(["M1"]), td([p(mention(uid(92), uid(93)))]), td(["Q1"])), tr(td(["M2"]), td(["unassigned"]), td(["Q2"]))),
    h(2, "Notes"),
    bq(p("Keep the scope small.")),
    ul(li(p("Item with ", t("code", "code")), ul(li("sub-item"))), li("Another")),
    code("ts", "export const x: number = 1;"),
    embed({ id: uid(94), entity_identifier: uid(95), project_identifier: uid(96), workspace_identifier: uid(97), entity_name: "issue" }),
    p("Done ", emoji("tada"))
  ));
  add("real-meeting-notes", "meeting-notes style document", doc(
    h(1, "Weekly sync"),
    pa("right", "2026-10-18"),
    h(2, "Attendees"),
    ul(li(p(mention(uid(100), uid(101)))), li(p(mention(uid(102), uid(103)))), li(p(mention(uid(104), uid(105))))),
    h(2, "Actions"),
    tl(ti(false, p("Ship the ", t("spike", "bold"))), ti(true, "Write the ADR")),
    h(2, "Discussion"),
    p("We looked at ", t("latency", color({ color: "orange" })), " and ", t("cost", color({ backgroundColor: "peach" })), ".")
  ));

  // generated documents (fuzz-style coverage of combinations)
  for (let s = 1; s <= 40; s++) add(`gen-small-${s}`, `generated document seed ${s}`, randomDoc(1000 + s, 3 + (s % 5)));
  for (let s = 1; s <= 12; s++) add(`gen-medium-${s}`, `generated document (medium) seed ${s}`, randomDoc(2000 + s, 25));
  for (let s = 1; s <= 3; s++) add(`gen-large-${s}`, `generated document (large) seed ${s}`, randomDoc(3000 + s, 300));
  add("gen-huge", "generated document: 2000 blocks (large-document CPU case)", randomDoc(4000, 2000));
  add("many-paragraphs", "3000 plain paragraphs of ~80 characters", doc(...Array.from({ length: 3000 }, (_, i) => p(`Paragraph ${i}: ` + sentence(rng(i + 7), 10)))));

  return D;
}

// ---- HTML that other writers (API v1 clients) send: no editor involved --------------------------
export type ForeignHtml = { id: string; desc: string; html: string; variant?: "rich" | "document" };
export function foreignHtml(): ForeignHtml[] {
  const F: ForeignHtml[] = [];
  const add = (id: string, desc: string, html: string, variant?: "rich" | "document") => F.push({ id, desc, html, ...(variant ? { variant } : {}) });
  add("f-p", "bare paragraph", "<p>Hello world</p>");
  add("f-text-only", "text with no block wrapper", "just some text");
  add("f-two-p", "two paragraphs", "<p>One</p><p>Two</p>");
  add("f-empty", "empty string", "");
  add("f-empty-p", "empty paragraph", "<p></p>");
  add("f-h", "headings h1-h6", "<h1>a</h1><h2>b</h2><h3>c</h3><h4>d</h4><h5>e</h5><h6>f</h6>");
  add("f-strong-em", "strong/em/b/i/s/del/u", "<p><strong>a</strong><b>b</b><em>c</em><i>d</i><s>e</s><del>f</del><strike>g</strike><u>h</u></p>");
  add("f-style-bold", "bold via font-weight style", '<p><span style="font-weight: bold">x</span><span style="font-weight: 700">y</span><span style="font-weight: 400">z</span></p>');
  add("f-style-italic", "italic via font-style", '<p><span style="font-style: italic">x</span></p>');
  add("f-style-deco", "underline/strike via text-decoration", '<p><span style="text-decoration: underline">u</span><span style="text-decoration: line-through">s</span></p>');
  add("f-a", "anchors", '<p><a href="https://example.com">x</a> <a href="mailto:a@b.c">m</a> <a href="/rel">r</a> <a>no href</a></p>');
  add("f-a-js", "javascript: and data: links", '<p><a href="javascript:alert(1)">x</a><a href=" JaVaScRiPt:alert(1)">y</a><a href="data:text/html;base64,AAAA">z</a></p>');
  add("f-code", "inline code", "<p>use <code>cargo build</code> now</p>");
  add("f-pre", "pre without code", "<pre>line1\nline2</pre>");
  add("f-pre-code", "pre/code with language class", '<pre><code class="language-js">let a = 1;\nlet b = 2;</code></pre>');
  add("f-pre-code-ws", "pre keeps whitespace", "<pre><code>  indented\n\n  blank above</code></pre>");
  add("f-ul", "bullet list", "<ul><li>a</li><li>b</li></ul>");
  add("f-ul-p", "bullet list with paragraphs", "<ul><li><p>a</p></li><li><p>b</p></li></ul>");
  add("f-ol-start", "ordered list with start", '<ol start="3"><li>a</li><li>b</li></ol>');
  add("f-ol-type", "ordered list with type", '<ol type="a"><li>a</li></ol>');
  add("f-nested-ul", "nested lists", "<ul><li>a<ul><li>b<ul><li>c</li></ul></li></ul></li></ul>");
  add("f-nested-ul-sibling", "ul directly inside ul (invalid)", "<ul><li>a</li><ul><li>b</li></ul></ul>");
  add("f-li-text-and-block", "li with text then block", "<ul><li>text<p>para</p></li></ul>");
  add("f-li-inline-mix", "li with inline marks", "<ul><li><strong>b</strong> and <em>i</em></li></ul>");
  add("f-tasklist", "task list (Plane markup)", '<ul data-type="taskList"><li data-type="taskItem" data-checked="true"><p>done</p></li><li data-type="taskItem" data-checked="false"><p>todo</p></li></ul>');
  add("f-tasklist-label", "task list (GitHub-style input)", '<ul class="contains-task-list"><li><input type="checkbox" checked> done</li><li><input type="checkbox"> todo</li></ul>');
  add("f-quote", "blockquote", "<blockquote><p>q</p></blockquote>");
  add("f-quote-text", "blockquote with bare text", "<blockquote>bare</blockquote>");
  add("f-hr", "hr", "<p>a</p><hr><p>b</p>");
  add("f-hr-div", "hr as Plane div", '<p>a</p><div data-type="horizontalRule"><div></div></div><p>b</p>');
  add("f-br", "br variants", "<p>a<br>b<br/>c</p>");
  add("f-br-only", "p with only br", "<p><br></p>");
  add("f-img", "img", '<p><img src="https://example.com/x.png" alt="x"></p>');
  add("f-img-data", "img with data URI", '<p><img src="data:image/png;base64,AAAA" alt="x"></p>');
  add("f-img-bare", "img outside paragraph", '<img src="https://example.com/x.png">');
  add("f-img-comp", "image-component", '<image-component src="00000000-0000-4000-8000-000000000001" width="400px" height="auto" aspectratio="1.5" alignment="center" status="uploaded"></image-component>');
  add("f-img-comp-case", "image-component with camelCase attr", '<image-component src="00000000-0000-4000-8000-000000000002" aspectRatio="1.5"></image-component>');
  add("f-mention", "mention-component", '<p>hi <mention-component id="00000000-0000-4000-8000-000000000003" entity_identifier="00000000-0000-4000-8000-000000000004" entity_name="user_mention"></mention-component></p>');
  add("f-emoji", "emoji span", '<p><span data-type="emoji" data-name="rocket">🚀</span></p>');
  add("f-table", "table with tbody", "<table><tbody><tr><th>h</th></tr><tr><td>c</td></tr></tbody></table>");
  add("f-table-bare", "table without tbody", "<table><tr><th>h</th></tr><tr><td>c</td></tr></table>");
  add("f-table-thead", "table with thead/tbody", "<table><thead><tr><th>h</th></tr></thead><tbody><tr><td>c</td></tr></tbody></table>");
  add("f-table-span", "table with spans/colwidth", '<table><tr><td colspan="2" rowspan="2" colwidth="120">x</td></tr></table>');
  add("f-table-style", "table cell colours via style", '<table><tr><td style="background-color: #ff0000; color: #ffffff">x</td></tr></table>');
  add("f-callout", "callout block", '<div data-block-type="callout-component" data-logo-in-use="emoji"><p>note</p></div>');
  add("f-callout-text", "callout with bare text", '<div data-block-type="callout-component">bare</div>');
  add("f-embed", "work item embed", '<issue-embed-component entity_identifier="00000000-0000-4000-8000-000000000005" id="00000000-0000-4000-8000-000000000006" entity_name="issue"></issue-embed-component>', "document");
  add("f-color-span", "customColor spans", '<p><span data-text-color="gray">g</span><span data-background-color="peach">p</span><span data-text-color="#ff0000" style="color: #ff0000">h</span></p>');
  add("f-align", "text-align styles", '<p style="text-align: center">c</p><h2 style="text-align: right">r</h2><p style="text-align: justify">j</p>');
  add("f-div", "unknown div wrappers", "<div><p>inside div</p></div><div>bare in div</div>");
  add("f-span", "unknown span", "<p><span>plain span</span></p>");
  add("f-font", "font/center legacy tags", "<p><font color=red>x</font><center>c</center></p>");
  add("f-script", "script and style elements", "<p>a</p><script>alert(1)</script><style>p{color:red}</style><p>b</p>");
  add("f-iframe", "iframe/object/embed", '<iframe src="https://example.com"></iframe><object data="x"></object><p>kept</p>');
  add("f-comment", "html comments", "<p>a<!-- hidden -->b</p>");
  add("f-entities", "named and numeric entities", "<p>&lt;tag&gt; &amp; &quot;q&quot; &#39;s&#39; &nbsp;sp &copy; &#x1F600; &unknown;</p>");
  add("f-whitespace", "inter-element whitespace and newlines", "<p>a\n   b\n\n  c</p>\n\n<p>  d  </p>");
  add("f-nested-inline", "deeply nested inline", "<p><strong><em><u><s>deep</s></u></em></strong></p>");
  add("f-p-in-p", "p inside p (invalid)", "<p>a<p>b</p>c</p>");
  add("f-block-in-inline", "block inside inline (invalid)", "<p><strong><p>x</p></strong></p>");
  add("f-unclosed", "unclosed tags", "<p>one<p>two<ul><li>a<li>b");
  add("f-deep-div-30", "30 nested divs", "<div>".repeat(30) + "<p>deep</p>" + "</div>".repeat(30));
  add("f-big-list", "100 list items", "<ul>" + Array.from({ length: 100 }, (_, i) => `<li>item ${i}</li>`).join("") + "</ul>");
  add("f-attr-quote", "attributes with odd quoting", "<p><a href='https://example.com/?a=1&b=2'>x</a><a href=https://example.com/unquoted>y</a></p>");
  add("f-class-style", "Plane renderer classes preserved on input", '<p class="editor-paragraph-block">x</p><h1 class="editor-heading-block">y</h1>');
  add("f-uppercase-tags", "uppercase tags", "<P>A</P><UL><LI>b</LI></UL><STRONG>c</STRONG>");
  add("f-mixed-doc", "typical API client payload", '<h2>Title</h2><p>Text with <strong>bold</strong>, <em>italic</em> and a <a href="https://example.com">link</a>.</p><ul><li>one</li><li>two</li></ul><pre><code class="language-python">print(1)</code></pre>');
  return F;
}
