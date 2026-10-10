# FLit

Work on one song together, each in your own DAW. A song is an append-only
log of signed events plus a store of files addressed by hash; people and
machines are IDs writing to that log; every DAW is a bridge into it.

- Spec: `plan.md`. Hand-over for the Windows / FL Studio session: `HANDOFF.md`.
- Build: `cargo build --release -p flit-agent -p fake-session && cargo xtask bundle flit-plugin --release`
- Test: `cargo test --workspace`
- Install (dev): `tools/install/install-linux.sh` or `tools/install/install-windows.ps1`

Layout: `crates/` (core, host trait, audio codecs, net, ipc, agent, plugin),
`bridges/` (fake, fl), `tools/` (fake-session CLI, installers), `fuzz/`.
