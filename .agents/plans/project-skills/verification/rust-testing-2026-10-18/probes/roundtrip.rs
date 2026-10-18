// Port of JS Math.round from the zip's testing-parity: let r = x.round(); if (r - x).abs() == 0.5 { x.ceil() } else { r }
fn js_round(x: f64) -> f64 {
    let r = x.round();
    if (r - x).abs() == 0.5 { x.ceil() } else { r }
}

fn main() {
    let xs = [-2.5f64, -1.5, -0.5, -0.4, 0.4, 0.5, 1.5, 2.5, 2.4, 2.6, -2.6];
    for x in xs {
        println!("round {x} std={} js_port={}", x.round(), js_round(x));
    }
    println!("fmt {:.0} {:.2} {:.0} {:.0} {:.2}", 2.5f64, 0.125f64, 0.5f64, 1.5f64, 0.375f64);
    println!("fmt1e21 {}", 1e21f64);
    println!("utf8len {} chars {} utf16 {}", "😀".len(), "😀".chars().count(), "😀".encode_utf16().count());
}
