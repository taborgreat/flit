#![no_main]
//! Opus and FLAC chunk decoders, and live stream packets, must never panic
//! on hostile input.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = flit_audio::decode_opus_chunk(data, None);
    let _ = flit_audio::decode_opus_chunk(data, Some(44_100));
    let _ = flit_audio::decode_flac_chunk(data);
    if let Some(p) = flit_audio::StreamPacket::from_bytes(data) {
        if let Ok(mut d) = flit_audio::StreamDecoder::new(flit_audio::StreamConfig::voice(48_000)) {
            let _ = d.decode(&p);
        }
    }
});
