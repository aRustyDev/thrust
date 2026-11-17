#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = thrust_preview_provisioner::password(data);
    if let Ok(path) = std::str::from_utf8(data) {
        let _ = thrust_preview_provisioner::route("PUT", path);
    }
});
