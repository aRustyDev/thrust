#![no_main]
use libfuzzer_sys::fuzz_target;
use thrust_adapters_postgres::preview_artifact::Artifact;

fuzz_target!(|data: &[u8]| {
    let _ = Artifact::parse(data, "0123456789abcdef0123456789abcdef01234567");
});
