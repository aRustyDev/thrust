#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = thrust_edge::fuzz_harness::pin_open(data);
});
