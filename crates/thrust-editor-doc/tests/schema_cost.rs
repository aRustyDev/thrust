use std::time::Instant;
use thrust_editor_doc::schema::{Schema, Variant};

/// `Schema::new` builds the content DFAs and attribute tables on every call; a Worker should build it once per isolate.
#[test]
fn schema_construction_cost() {
    let t = Instant::now();
    let n = 2000;
    for _ in 0..n {
        std::hint::black_box(Schema::new(Variant::Document));
    }
    let per = t.elapsed().as_secs_f64() * 1e6 / n as f64;
    eprintln!("Schema::new(Document): {per:.1} us each ({}) -- build once per isolate", if cfg!(debug_assertions) { "debug build" } else { "release build" });
}
