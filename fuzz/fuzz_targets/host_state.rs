#![no_main]
//! HostState passthrough: any blob goes in, the same blob comes out of
//! the fold, and the chains never panic on it.
use flit_core::*;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let key = Keypair::from_seed(1);
    let host = HostTag::new(if data.first().map(|b| b % 2 == 0).unwrap_or(true) { "a" } else { "b" });
    let who = Stamper::new(key, "fuzz", host.clone(), "0.1.0", "m");
    let dir = std::env::temp_dir().join(format!("flit-fuzz-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut song = Song::create(&dir, &who, "fuzz", 0).unwrap();
    let track = TrackId::new("t");
    match song.host_state(&who, main_history(), &track, data.to_vec(), 1) {
        Ok(_) => {
            let st = song.main();
            assert_eq!(st.track(&track).unwrap().host_state.get(&host).map(|v| v.as_slice()), Some(data));
        }
        Err(Error::Invalid(_)) => assert!(data.len() > flit_core::seal::verify::MAX_HOST_STATE),
        Err(e) => panic!("{e}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
});
