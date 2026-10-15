#!/usr/bin/env bash
# Builds the shared-note renderer (crates/kolada-share-render) as
# WebAssembly and puts it where the notsy.co Worker loads it from:
# services/notsy/wasm/kolada_share_render.wasm.
#
#   tools/build-share-wasm.sh
#
# Needs the wasm32-unknown-unknown target (rustup target add
# wasm32-unknown-unknown). Run from anywhere; it works from the repo root.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build -p kolada-share-render --target wasm32-unknown-unknown --profile share-wasm --target-dir target/wasm
mkdir -p services/notsy/wasm
cp target/wasm/wasm32-unknown-unknown/share-wasm/kolada_share_render.wasm services/notsy/wasm/kolada_share_render.wasm
echo "services/notsy/wasm/kolada_share_render.wasm: $(wc -c < services/notsy/wasm/kolada_share_render.wasm) bytes"
