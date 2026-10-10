#![no_main]
//! SysEx from the FL script side must never panic the bridge.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut c = flit_bridge_fl::Chunker::default();
    for m in data.split(|b| *b == 0xF7) {
        if let Some(f) = c.push(m) {
            let _ = serde_json::from_str::<flit_bridge_fl::FlReply>(&f.json);
            let _ = serde_json::from_str::<flit_bridge_fl::FlEvent>(&f.json);
        }
    }
});
