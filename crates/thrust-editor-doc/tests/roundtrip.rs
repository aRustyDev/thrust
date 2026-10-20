//! Round trips of generated documents through the three conversions, hermetic (gap G-10 in
//! `docs/src/testing/status.md`). The golden corpus (`parity_`) and the TypeScript differential runs (`oracle_`) check
//! the conversions against Plane on recorded and mutated inputs; these check them against themselves on documents
//! nobody wrote down:
//!
//! * JSON: `Node::from_json(to_json(d)) == d`;
//! * Yjs: `ydoc::decode(ydoc::encode(d)) == d` (compared as JSON with numbers normalised, as `parity_stage1` does);
//! * HTML is not an identity: ProseMirror's parser collapses whitespace and normalises structure, in TypeScript too.
//!   So the properties are that one round trip reaches a fixed point (`parse(to_html(parse(to_html(d))))` equals
//!   `parse(to_html(d))`) and that it loses no text except whitespace;
//! * decoding a mutated Yjs update and parsing arbitrary tag soup never panic (a panic resets a Worker isolate).
//!
//! Documents are generated as a small spec (blocks, list items, tables, inline runs with marks) and built with the
//! crate's own constructors: `Mark::add_to_set` orders marks and applies their exclusions, `join_text` merges adjacent
//! text with equal marks. Every generated document must pass `Node::check`, so the generator cannot drift out of the
//! schema unnoticed. Both schema variants are used.

mod common;

use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;
use serde_json::{json, Value};
use thrust_editor_doc::{
    domparse, html,
    pm::{self, Attrs, Mark, Node},
    schema::{Schema, Variant},
    ydoc,
};

// ---------------------------------------------------------------------------------------------
// The document spec and its generator
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum M {
    Bold,
    Italic,
    Strike,
    Underline,
    Code,
    /// One of a few safe URLs.
    Link(u8),
}

#[derive(Debug, Clone)]
enum Inline {
    Text { text: String, marks: Vec<M> },
    Break,
}

/// A list item: its first paragraph, then any blocks.
type Item = (Vec<Inline>, Vec<Block>);

#[derive(Debug, Clone)]
enum Block {
    Paragraph { align: Option<u8>, inline: Vec<Inline> },
    Heading { level: u8, inline: Vec<Inline> },
    Bullet(Vec<Item>),
    Ordered { start: u8, items: Vec<Item> },
    Tasks(Vec<(bool, Item)>),
    Quote(Vec<Block>),
    Code { lang: Option<u8>, text: String },
    Rule,
    /// Rows of cells; a cell is a header or not, with its blocks.
    Table(Vec<Vec<(bool, Vec<Block>)>>),
}

fn mark() -> impl Strategy<Value = M> {
    prop_oneof![
        Just(M::Bold),
        Just(M::Italic),
        Just(M::Strike),
        Just(M::Underline),
        Just(M::Code),
        (0..3u8).prop_map(M::Link),
    ]
}

/// Text with the characters HTML escaping and whitespace handling care about.
fn text() -> impl Strategy<Value = String> {
    "[a-z <>&\"'é😀]{1,8}"
}

fn inline() -> impl Strategy<Value = Vec<Inline>> {
    prop::collection::vec(
        prop_oneof![
            6 => (text(), prop::collection::vec(mark(), 0..3)).prop_map(|(text, marks)| Inline::Text { text, marks }),
            1 => Just(Inline::Break),
        ],
        0..5,
    )
}

fn leaf() -> impl Strategy<Value = Block> {
    prop_oneof![
        6 => (prop::option::of(0..3u8), inline()).prop_map(|(align, inline)| Block::Paragraph { align, inline }),
        2 => (1..=6u8, inline()).prop_map(|(level, inline)| Block::Heading { level, inline }),
        1 => (prop::option::of(0..2u8), "[a-z <>&\n]{0,12}").prop_map(|(lang, text)| Block::Code { lang, text }),
        1 => Just(Block::Rule),
    ]
}

fn blocks() -> impl Strategy<Value = Vec<Block>> {
    let block = leaf().prop_recursive(3, 24, 3, |inner| {
        let item = || (inline(), prop::collection::vec(inner.clone(), 0..2));
        prop_oneof![
            2 => prop::collection::vec(item(), 1..3).prop_map(Block::Bullet),
            1 => (1..4u8, prop::collection::vec(item(), 1..3)).prop_map(|(start, items)| Block::Ordered { start, items }),
            1 => prop::collection::vec((any::<bool>(), item()), 1..3).prop_map(Block::Tasks),
            1 => prop::collection::vec(inner.clone(), 1..3).prop_map(Block::Quote),
            1 => prop::collection::vec(
                prop::collection::vec((any::<bool>(), prop::collection::vec(inner, 1..2)), 1..3),
                1..3
            )
            .prop_map(Block::Table),
        ]
    });
    prop::collection::vec(block, 1..5)
}

// ---------------------------------------------------------------------------------------------
// Building documents with the crate's constructors
// ---------------------------------------------------------------------------------------------

fn attrs(v: Value) -> Attrs {
    v.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn node(schema: &Schema, name: &str, a: Value, content: Vec<Node>) -> Node {
    Node::create(schema, schema.node_type(name).unwrap(), &attrs(a), content)
}

fn marks(schema: &Schema, ms: &[M]) -> Vec<Mark> {
    ms.iter().fold(Vec::new(), |set, m| {
        let (name, a) = match m {
            M::Bold => ("bold", json!({})),
            M::Italic => ("italic", json!({})),
            M::Strike => ("strike", json!({})),
            M::Underline => ("underline", json!({})),
            M::Code => ("code", json!({})),
            M::Link(n) => ("link", json!({ "href": format!("https://example.com/{n}") })),
        };
        Mark::create(schema, schema.mark_type(name).unwrap(), &attrs(a)).add_to_set(schema, &set)
    })
}

fn inlines(schema: &Schema, xs: &[Inline]) -> Vec<Node> {
    let nodes = xs
        .iter()
        .map(|x| match x {
            Inline::Text { text, marks: ms } => Node::text(schema, text, marks(schema, ms)).unwrap(),
            Inline::Break => node(schema, "hardBreak", json!({}), vec![]),
        })
        .collect();
    pm::join_text(nodes)
}

fn item(schema: &Schema, name: &str, a: Value, (first, rest): &Item) -> Node {
    let mut content = vec![node(schema, "paragraph", json!({}), inlines(schema, first))];
    content.extend(rest.iter().map(|b| block(schema, b)));
    node(schema, name, a, content)
}

fn block(schema: &Schema, b: &Block) -> Node {
    const ALIGN: [&str; 3] = ["left", "center", "right"];
    const LANG: [&str; 2] = ["rust", "typescript"];
    match b {
        Block::Paragraph { align, inline } => node(
            schema,
            "paragraph",
            json!({ "textAlign": align.map(|a| ALIGN[a as usize]) }),
            inlines(schema, inline),
        ),
        Block::Heading { level, inline } => node(schema, "heading", json!({ "level": level }), inlines(schema, inline)),
        Block::Bullet(items) => {
            node(schema, "bulletList", json!({}), items.iter().map(|i| item(schema, "listItem", json!({}), i)).collect())
        }
        Block::Ordered { start, items } => node(
            schema,
            "orderedList",
            json!({ "start": start }),
            items.iter().map(|i| item(schema, "listItem", json!({}), i)).collect(),
        ),
        Block::Tasks(items) => node(
            schema,
            "taskList",
            json!({}),
            items.iter().map(|(checked, i)| item(schema, "taskItem", json!({ "checked": checked }), i)).collect(),
        ),
        Block::Quote(bs) => node(schema, "blockquote", json!({}), bs.iter().map(|b| block(schema, b)).collect()),
        Block::Code { lang, text } => {
            let content = if text.is_empty() { vec![] } else { vec![Node::text(schema, text, vec![]).unwrap()] };
            node(schema, "codeBlock", json!({ "language": lang.map(|l| LANG[l as usize]) }), content)
        }
        Block::Rule => node(schema, "horizontalRule", json!({}), vec![]),
        Block::Table(rows) => node(
            schema,
            "table",
            json!({}),
            rows.iter()
                .map(|cells| {
                    let cells = cells
                        .iter()
                        .map(|(header, bs)| {
                            let name = if *header { "tableHeader" } else { "tableCell" };
                            node(schema, name, json!({}), bs.iter().map(|b| block(schema, b)).collect())
                        })
                        .collect();
                    node(schema, "tableRow", json!({}), cells)
                })
                .collect(),
        ),
    }
}

/// The document a spec describes, checked against the schema.
fn build(schema: &Schema, spec: &[Block]) -> Node {
    let d = pm::doc(schema, spec.iter().map(|b| block(schema, b)).collect());
    d.check(schema).unwrap_or_else(|e| panic!("the generator built an invalid document: {e}\n{spec:#?}"));
    d
}

fn schema(rich: bool) -> Schema {
    Schema::new(if rich { Variant::Rich } else { Variant::Document })
}

fn json_of(schema: &Schema, d: &Node) -> Value {
    pm::norm_num(d.to_json(schema))
}

/// Every text run of the spec, in document order: what `text_content` must return.
fn spec_text(spec: &[Block]) -> String {
    fn inl(xs: &[Inline], out: &mut String) {
        for x in xs {
            if let Inline::Text { text, .. } = x {
                out.push_str(text);
            }
        }
    }
    fn blk(bs: &[Block], out: &mut String) {
        for b in bs {
            match b {
                Block::Paragraph { inline, .. } | Block::Heading { inline, .. } => inl(inline, out),
                Block::Bullet(items) | Block::Ordered { items, .. } => {
                    for (first, rest) in items {
                        inl(first, out);
                        blk(rest, out);
                    }
                }
                Block::Tasks(items) => {
                    for (_, (first, rest)) in items {
                        inl(first, out);
                        blk(rest, out);
                    }
                }
                Block::Quote(bs) => blk(bs, out),
                Block::Code { text, .. } => out.push_str(text),
                Block::Rule => {}
                Block::Table(rows) => {
                    for (_, bs) in rows.iter().flatten() {
                        blk(bs, out);
                    }
                }
            }
        }
    }
    let mut out = String::new();
    blk(spec, &mut out);
    out
}

fn without_whitespace(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

// ---------------------------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(common::proptest_config(256))]

    #[test]
    fn prop_json_round_trip_is_the_identity(spec in blocks(), rich in any::<bool>()) {
        let schema = schema(rich);
        let d = build(&schema, &spec);
        let back = Node::from_json(&schema, &d.to_json(&schema)).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(back, d);
    }

    #[test]
    fn prop_yjs_round_trip_is_the_identity(spec in blocks(), rich in any::<bool>(), client in 1..u32::MAX) {
        let schema = schema(rich);
        let d = build(&schema, &spec);
        let update = ydoc::encode(&schema, &d, None, Some(u64::from(client)));
        let back = ydoc::decode(&schema, &update, "default").map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(json_of(&schema, &back), json_of(&schema, &d));
    }

    /// The title is written by a second Yjs client and merged into the body's update: both must come back, for any
    /// client id (yrs requires 53-bit ids, a JavaScript-safe integer, and checks it in debug builds), which also
    /// shows the two clients are distinct (equal ids would clash in the merge).
    #[test]
    fn prop_yjs_body_and_title_round_trip_together(spec in blocks(), title in blocks(), client in any::<u64>()) {
        let schema = schema(false);
        let (d, t) = (build(&schema, &spec), build(&schema, &title));
        let update = ydoc::encode(&schema, &d, Some(&t), Some(client));
        let body = ydoc::decode(&schema, &update, "default").map_err(|e| TestCaseError::fail(e.to_string()))?;
        let back = ydoc::decode(&schema, &update, "title").map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(json_of(&schema, &body), json_of(&schema, &d));
        prop_assert_eq!(json_of(&schema, &back), json_of(&schema, &t));
    }

    /// `Node::check` must reject what the schema forbids; generated documents alone never exercise that.
    #[test]
    fn prop_check_rejects_broken_documents(spec in blocks(), breakage in 0..4u8, at in any::<prop::sample::Index>()) {
        let schema = schema(false);
        let mut d = build(&schema, &spec);
        let p = |content| node(&schema, "paragraph", json!({}), content);
        let bad = match breakage {
            // doc: block+, not text
            0 => Node::text(&schema, "loose", vec![]).unwrap(),
            // bulletList: listItem+
            1 => node(&schema, "bulletList", json!({}), vec![]),
            // paragraph: inline*
            2 => p(vec![p(vec![])]),
            // listItem: paragraph block*, so it must start with a paragraph
            _ => node(&schema, "bulletList", json!({}), vec![node(&schema, "listItem", json!({}), vec![
                node(&schema, "horizontalRule", json!({}), vec![]),
            ])]),
        };
        let i = at.index(d.content.len() + 1);
        d.content.insert(i, bad);
        prop_assert!(d.check(&schema).is_err(), "breakage {} at {} was accepted", breakage, i);
    }

    #[test]
    fn prop_html_round_trip_reaches_a_fixed_point(spec in blocks(), rich in any::<bool>()) {
        let schema = schema(rich);
        let d = build(&schema, &spec);
        let once = domparse::html_to_doc(&schema, &html::to_html(&schema, &d));
        let twice = domparse::html_to_doc(&schema, &html::to_html(&schema, &once));
        prop_assert_eq!(json_of(&schema, &twice), json_of(&schema, &once), "html: {}", html::to_html(&schema, &once));
    }

    #[test]
    fn prop_text_content_is_the_spec_text_in_order(spec in blocks(), rich in any::<bool>()) {
        let schema = schema(rich);
        prop_assert_eq!(build(&schema, &spec).text_content(), spec_text(&spec));
    }

    /// The expected text comes from the spec, not from `text_content` on both sides (which a broken
    /// `text_content` would pass).
    #[test]
    fn prop_html_round_trip_loses_no_text(spec in blocks(), rich in any::<bool>()) {
        let schema = schema(rich);
        let d = build(&schema, &spec);
        let once = domparse::html_to_doc(&schema, &html::to_html(&schema, &d));
        prop_assert_eq!(without_whitespace(&once.text_content()), without_whitespace(&spec_text(&spec)));
    }

    #[test]
    fn prop_decode_never_panics_on_mutated_updates(
        spec in blocks(),
        edits in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>(), 0..3u8), 1..8),
    ) {
        let schema = schema(false);
        let mut update = ydoc::encode(&schema, &build(&schema, &spec), None, Some(42));
        for (at, byte, kind) in &edits {
            let i = at.index(update.len().max(1));
            match kind {
                0 if !update.is_empty() => update[i] = *byte,
                1 => update.insert(i.min(update.len()), *byte),
                _ if !update.is_empty() => { update.truncate(i); }
                _ => {}
            }
        }
        let outcome = catch_unwind(AssertUnwindSafe(|| ydoc::decode(&schema, &update, "default").map(|_| ())));
        prop_assert!(outcome.is_ok(), "decode panicked on {:?}", update);
    }

    #[test]
    fn prop_html_to_doc_never_panics(
        soup in prop::collection::vec(
            prop_oneof![
                Just("<p>"), Just("</p>"), Just("<ul><li>"), Just("</li></ul>"), Just("<table><tr><td>"),
                Just("</td></tr></table>"), Just("<pre><code>"), Just("</code></pre>"), Just("<a href=\"x\">"),
                Just("</a>"), Just("<strong>"), Just("<br>"), Just("<img src=x>"), Just("&amp;"), Just("&#x1F600;"),
                Just("<"), Just(">"), Just("\""), Just("text"), Just(" "), Just("\n"),
            ],
            0..40,
        ),
        rich in any::<bool>(),
    ) {
        let schema = schema(rich);
        let input: String = soup.concat();
        let outcome = catch_unwind(AssertUnwindSafe(|| domparse::html_to_doc(&schema, &input)));
        prop_assert!(outcome.is_ok(), "html_to_doc panicked on {:?}", input);
    }
}

// ---------------------------------------------------------------------------------------------
// Generator coverage (Rule 8)
// ---------------------------------------------------------------------------------------------

fn kinds(bs: &[Block], seen: &mut BTreeSet<String>) {
    let inline_kinds = |xs: &[Inline], seen: &mut BTreeSet<String>| {
        for x in xs {
            match x {
                Inline::Text { marks, .. } => {
                    seen.insert("text".into());
                    for m in marks {
                        seen.insert(format!("{:?}", m).split('(').next().unwrap().to_string());
                    }
                    if marks.len() > 1 {
                        seen.insert("several marks".into());
                    }
                }
                Inline::Break => {
                    seen.insert("hardBreak".into());
                }
            }
        }
    };
    for b in bs {
        match b {
            Block::Paragraph { inline, .. } => {
                seen.insert(if inline.is_empty() { "empty paragraph" } else { "paragraph" }.into());
                inline_kinds(inline, seen);
            }
            Block::Heading { inline, .. } => {
                seen.insert("heading".into());
                inline_kinds(inline, seen);
            }
            Block::Bullet(items) | Block::Ordered { items, .. } => {
                seen.insert(if matches!(b, Block::Bullet(_)) { "bulletList" } else { "orderedList" }.into());
                for (first, rest) in items {
                    inline_kinds(first, seen);
                    if !rest.is_empty() {
                        seen.insert("nested block in a list item".into());
                    }
                    kinds(rest, seen);
                }
            }
            Block::Tasks(items) => {
                seen.insert("taskList".into());
                for (_, (first, rest)) in items {
                    inline_kinds(first, seen);
                    kinds(rest, seen);
                }
            }
            Block::Quote(bs) => {
                seen.insert("blockquote".into());
                kinds(bs, seen);
            }
            Block::Code { text, .. } => {
                seen.insert(if text.contains('\n') { "multi-line codeBlock" } else { "codeBlock" }.into());
            }
            Block::Rule => {
                seen.insert("horizontalRule".into());
            }
            Block::Table(rows) => {
                seen.insert("table".into());
                for (header, bs) in rows.iter().flatten() {
                    seen.insert(if *header { "tableHeader" } else { "tableCell" }.into());
                    kinds(bs, seen);
                }
            }
        }
    }
}

/// The generator reaches every node kind, every mark, stacked marks and nested blocks.
#[test]
fn generated_documents_reach_every_kind() {
    const ALL: [&str; 24] = [
        "paragraph", "empty paragraph", "heading", "bulletList", "orderedList", "taskList", "blockquote", "codeBlock",
        "multi-line codeBlock", "horizontalRule", "table", "tableHeader", "tableCell", "text", "hardBreak", "Bold",
        "Italic", "Strike", "Underline", "Code", "Link", "several marks", "nested block in a list item",
        // a document nests at least one container in another
        "deep",
    ];
    let mut runner = TestRunner::deterministic();
    let mut seen = BTreeSet::new();
    for _ in 0..500 {
        let spec = blocks().new_tree(&mut runner).unwrap().current();
        let nested = spec.iter().any(|b| match b {
            Block::Quote(bs) => bs.iter().any(|b| !matches!(b, Block::Paragraph { .. } | Block::Heading { .. })),
            _ => false,
        });
        if nested {
            seen.insert("deep".to_string());
        }
        kinds(&spec, &mut seen);
    }
    let missing: Vec<_> = ALL.iter().filter(|k| !seen.contains(**k)).collect();
    assert!(missing.is_empty(), "never generated: {missing:?}");
}

// ---------------------------------------------------------------------------------------------
// Dangerous link targets (X12)
// ---------------------------------------------------------------------------------------------

/// What browsers skip before a URL scheme: JavaScript whitespace and the C0 controls (`isDangerousHref`).
const SKIPPED: [char; 16] = [
    ' ', '\t', '\n', '\r', '\u{b}', '\u{c}', '\u{a0}', '\u{feff}', '\u{3000}', '\u{0}', '\u{1}', '\u{8}', '\u{e}', '\u{1f}',
    '\u{2028}', '\u{200a}',
];

/// A `javascript:`, `data:` or `vbscript:` URL disguised the ways an attacker would: mixed case, skipped characters
/// in front, and tabs or newlines inside the scheme (browsers drop those).
fn dangerous_href() -> impl Strategy<Value = String> {
    (
        prop::sample::select(vec!["javascript:", "data:", "vbscript:"]),
        prop::collection::vec(any::<bool>(), 12),
        prop::collection::vec(prop::sample::select(SKIPPED.to_vec()), 0..4),
        prop::collection::vec((any::<prop::sample::Index>(), prop::sample::select(vec!['\t', '\n', '\r'])), 0..3),
        "[a-z()0-9,/]{0,10}",
    )
        .prop_map(|(scheme, upper, prefix, inside, rest)| {
            let mut s: String =
                scheme.chars().zip(upper).map(|(c, up)| if up { c.to_ascii_uppercase() } else { c }).collect();
            for (at, c) in inside {
                let i = at.index(s.len());
                s.insert(i, c);
            }
            prefix.into_iter().collect::<String>() + &s + &rest
        })
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    #[test]
    fn prop_dangerous_hrefs_are_recognised_however_disguised(href in dangerous_href()) {
        prop_assert!(html::is_dangerous_href(&href), "{:?} was not recognised", href);
    }

    #[test]
    fn prop_ordinary_hrefs_are_not_dangerous(
        prefix in prop::collection::vec(prop::sample::select(SKIPPED.to_vec()), 0..3),
        href in prop_oneof![
            "https://[a-z]{1,8}\\.example/[a-z]{0,6}", "mailto:[a-z]{1,6}@example\\.com", "/[a-z/]{0,10}", "#[a-z]{1,6}",
            "javascript[a-z]{0,4}", "[a-z]{1,6}:javascript:x",
        ],
    ) {
        let href = prefix.into_iter().collect::<String>() + &href;
        prop_assert!(!html::is_dangerous_href(&href), "{:?} was called dangerous", href);
    }

    /// Judged without `is_dangerous_href` (a broken one would pass its own output): the only links in the rendered
    /// HTML are the safe ones the document had, and the dangerous one renders with an empty `href`.
    #[test]
    fn prop_rendered_links_never_carry_a_dangerous_href(spec in blocks(), href in dangerous_href(), rich in any::<bool>()) {
        let schema = schema(rich);
        let mut d = build(&schema, &spec);
        let link = Mark::create(&schema, schema.mark_type("link").unwrap(), &attrs(json!({ "href": href })));
        let text = Node::text(&schema, "click", vec![link]).unwrap();
        d.content.push(node(&schema, "paragraph", json!({}), vec![text]));
        let out = html::to_html(&schema, &d);
        let hrefs: Vec<&str> = out.split("href=\"").skip(1).map(|s| s.split('"').next().unwrap()).collect();
        prop_assert!(hrefs.contains(&""), "the dangerous link lost its empty href: {}", out);
        for h in hrefs {
            prop_assert!(h.is_empty() || h.starts_with("https://example.com/"), "rendered href {:?} in {}", h, out);
        }
    }
}
