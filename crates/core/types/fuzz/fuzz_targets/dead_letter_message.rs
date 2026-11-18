#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = thrust_contracts::fuzz_harness::dead_letter_message(data);
});
