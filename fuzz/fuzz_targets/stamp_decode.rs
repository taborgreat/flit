#![no_main]
//! Act and fact decoding (binary and JSON) and the hash tier must never panic.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(a) = flit_core::Act::from_bytes(data) {
        let _ = a.check_id();
        let _ = a.signed_by(&a.name());
        let _ = a.to_json();
        let _ = a.payload().property();
    }
    if let Ok(s) = std::str::from_utf8(data) {
        if let Ok(a) = flit_core::Act::from_json(s) {
            let _ = a.check_id();
            let _ = a.to_bytes();
        }
        if let Ok(f) = flit_core::Fact::from_json(s) {
            let _ = f.verify();
        }
    }
});
