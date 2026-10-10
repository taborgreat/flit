#![no_main]
//! IPC frames from a (possibly hostile) local client must never panic.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(f) = flit_ipc::framing::decode::<flit_ipc::Frame>(data) {
        let _ = flit_ipc::framing::encode(&f);
    }
    let _ = flit_ipc::framing::decode::<flit_ipc::Request>(data);
    let _ = flit_ipc::framing::decode::<flit_ipc::Push>(data);
});
