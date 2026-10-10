#![no_main]
//! Stamp decoding (binary and JSON) and the hash tier must never panic.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(st) = flit_core::Stamp::from_bytes(data) {
        let _ = st.verify();
        let _ = st.to_json();
        let _ = st.moment_key();
    }
    if let Ok(s) = std::str::from_utf8(data) {
        if let Ok(st) = flit_core::Stamp::from_json(s) {
            let _ = st.verify();
            let _ = st.to_bytes();
        }
    }
});
