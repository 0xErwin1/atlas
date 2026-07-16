#!/usr/bin/env bash
set -euo pipefail

root=$(git rev-parse --show-toplevel)
target_dir=$(mktemp -d)

cleanup() {
    rm -rf "$target_dir"
}
trap cleanup EXIT

cd "$root"
CARGO_TARGET_DIR="$target_dir" cargo build --quiet --release -p atlas_desktop --bin atlas_desktop

binary="$target_dir/release/atlas_desktop"
test -x "$binary"
test ! -e "$target_dir/release/atlas-desktop-gate"

python3 - "$root/apps/desktop/src-tauri/Cargo.toml" "$root/apps/desktop/src-tauri/tauri.conf.json" "$binary" <<'PY'
import pathlib
import sys

manifest = pathlib.Path(sys.argv[1]).read_text()
tauri_config = pathlib.Path(sys.argv[2]).read_text()
binary = pathlib.Path(sys.argv[3]).read_bytes()

assert 'required-features = ["desktop-gate"]' in manifest
assert '"frontendDist": "../../web/dist"' in tauri_config

for forbidden in (
    b'atlas-desktop-gate',
    b'GateTransportFactory',
    b'TlsGateServer',
    b'atlas_test_db',
    b'rcgen',
    b'desktop-gate',
    b'ATLAS_DESKTOP_GATE_CA_PATH',
    b'desktop-gate is a non-shipping target',
    b'danger_accept_invalid_certs',
    b'tls_certs_merge',
):
    assert forbidden not in binary, forbidden
PY
