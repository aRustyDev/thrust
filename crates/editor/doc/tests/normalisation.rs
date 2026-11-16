//! Properties of the small normalisations the conversions rely on (gap G-10 in `docs/src/testing/status.md`).
//! Mutation testing of `src/pm.rs` and `src/html.rs` showed that the round trips cannot see them: the round-trip
//! generator builds mark sets with `Mark::add_to_set` itself, and both sides of a comparison go through `norm_num`.
//!
//! * `Mark::add_to_set` (ProseMirror `Mark.addToSet`): the set is ordered by rank with one mark per type, the last
//!   `link` wins, and `code` excludes every other mark (Tiptap's code mark has `excludes: "_"`), in whatever order
//!   the marks arrive;
//! * `pm::norm_num`: an integral float below 2^53 becomes the integer, anything else is unchanged, at any depth;
//! * `html::js_number_string` (JavaScript `Number.prototype.toString`): it parses back to the same number, and uses
//!   exponent form exactly when JavaScript does (|n| >= 1e21, or 0 < |n| < 1e-6), with an explicit `+` for positive
//!   exponents.

mod common;

use proptest::prelude::*;
use serde_json::{json, Number, Value};
use thrust_editor_doc::{
    html::js_number_string,
    pm::{self, Attrs, Mark},
    schema::{Schema, Variant},
};

const MARKS: [&str; 7] = ["bold", "italic", "strike", "underline", "code", "textStyle", "link"];

fn mark(schema: &Schema, name: &str, href: u8) -> Mark {
    let attrs: Attrs = if name == "link" {
        [("href".to_string(), json!(format!("https://example.com/{href}")))].into_iter().collect()
    } else {
        Attrs::new()
    };
    Mark::create(schema, schema.mark_type(name).unwrap(), &attrs)
}

/// Doubles that matter for number formatting: the exponent thresholds, 2^53, integers, fractions, tiny and huge.
fn number() -> impl Strategy<Value = f64> {
    prop_oneof![
        3 => any::<f64>().prop_filter("finite", |f| f.is_finite()),
        2 => (-1_000_000i64..1_000_000).prop_map(|i| i as f64),
        2 => (-1000i32..1000).prop_map(|i| f64::from(i) / 8.0),
        1 => prop::sample::select(vec![
            0.0, -0.0, 1e21, -1e21, 9.999_999_999_999_999e20, 1e-6, 1e-7, 9.9e-7, -1e-7, 1.5e-7, 1e300, 5e-324,
            9_007_199_254_740_992.0, 9_007_199_254_740_991.0, -9_007_199_254_740_992.0,
        ]),
    ]
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    #[test]
    fn prop_mark_sets_are_ordered_and_code_excludes_the_rest(
        names in prop::collection::vec((prop::sample::select(MARKS.to_vec()), 0..3u8), 0..8),
    ) {
        let schema = Schema::new(Variant::Document);
        let set = names.iter().fold(Vec::new(), |set, (n, h)| mark(&schema, n, *h).add_to_set(&schema, &set));
        let want: Vec<Mark> = if names.iter().any(|(n, _)| *n == "code") {
            vec![mark(&schema, "code", 0)]
        } else {
            // one mark per type, the last of each type, ordered by rank
            let mut last: Vec<(&str, u8)> = Vec::new();
            for (n, h) in &names {
                last.retain(|(m, _)| m != n);
                last.push((n, *h));
            }
            let mut want: Vec<Mark> = last.iter().map(|(n, h)| mark(&schema, n, *h)).collect();
            want.sort_by_key(|m| schema.marks[m.ty].rank);
            want
        };
        prop_assert_eq!(set, want);
    }

    #[test]
    fn prop_norm_num_turns_integral_floats_below_2_pow_53_into_integers(f in number()) {
        let n = Number::from_f64(f).unwrap();
        let want = if f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 {
            json!(f as i64)
        } else {
            Value::Number(n.clone())
        };
        // nested in arrays and objects too
        prop_assert_eq!(pm::norm_num(json!([{ "a": Value::Number(n) }])), json!([{ "a": want }]));
    }

    #[test]
    fn prop_js_number_string_parses_back_to_the_same_number(f in number()) {
        let s = js_number_string(&Number::from_f64(f).unwrap());
        let back: f64 = s.parse().map_err(|e| TestCaseError::fail(format!("{s:?}: {e}")))?;
        prop_assert!(back == f, "{} printed as {:?}, which reads back as {}", f, s, back);
    }

    #[test]
    fn prop_js_number_string_uses_exponent_form_where_javascript_does(f in number()) {
        let s = js_number_string(&Number::from_f64(f).unwrap());
        let exponent = f.abs() >= 1e21 || (f != 0.0 && f.abs() < 1e-6);
        prop_assert_eq!(s.contains('e'), exponent, "{} printed as {:?}", f, s);
        if exponent && f.abs() >= 1.0 {
            prop_assert!(s.contains("e+"), "a positive exponent is written e+: {:?}", s);
        }
        if !exponent && f.fract() == 0.0 {
            prop_assert!(!s.contains('.'), "an integral number has no decimal point: {:?}", s);
        }
    }
}
