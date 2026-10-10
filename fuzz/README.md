# Fuzz targets (addendum 2 §4)

Needs nightly + cargo-fuzz:

    rustup toolchain install nightly
    cargo install cargo-fuzz
    cd fuzz
    cargo +nightly fuzz run stamp_decode   -- -max_total_time=120
    cargo +nightly fuzz run chunk_decode   -- -max_total_time=120
    cargo +nightly fuzz run host_state     -- -max_total_time=120
    cargo +nightly fuzz run ipc_decode     -- -max_total_time=120
    cargo +nightly fuzz run sysex_decode   -- -max_total_time=120

The same decoders are also hit with random bytes by ordinary `cargo test`
(see `crates/*/tests/*smoke*` and the `garbage_*` unit tests), so a plain
stable toolchain still exercises the "never panic on garbage" rule.
