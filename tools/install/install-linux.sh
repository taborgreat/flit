#!/usr/bin/env bash
# FLit dev install for Linux (session 1 machine, FakeHost or plugin-only).
#   tools/install/install-linux.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
cargo build --release -p flit-agent -p fake-session
cargo xtask bundle flit-plugin --release
mkdir -p ~/.vst3 ~/.clap ~/.local/share/flit/bin
rm -rf ~/.vst3/FLit.vst3
cp -r target/bundled/FLit.vst3 ~/.vst3/
cp target/bundled/FLit.clap ~/.clap/
cp target/release/flit-agent target/release/fake-session ~/.local/share/flit/bin/
echo "Installed ~/.vst3/FLit.vst3, ~/.clap/FLit.clap, ~/.local/share/flit/bin/{flit-agent,fake-session}"
echo "The plugin starts the agent on demand (plugin-only, level 0). For a FakeHost session:"
echo "  ~/.local/share/flit/bin/flit-agent --bridge fake --local"
