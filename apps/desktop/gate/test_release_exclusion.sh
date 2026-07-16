#!/usr/bin/env bash
set -euo pipefail

root=$(git rev-parse --show-toplevel)
manifest="$root/apps/desktop/src-tauri/Cargo.toml"
gate_module="$root/apps/desktop/src-tauri/src/gate.rs"
tauri_config="$root/apps/desktop/src-tauri/tauri.conf.json"
launch_controller="$root/apps/desktop/gate/run_webdriver_launch.sh"
release_audit="$root/apps/desktop/gate/audit_release_exclusion.sh"
justfile="$root/justfile"

python3 - "$manifest" "$gate_module" "$tauri_config" "$launch_controller" "$release_audit" "$justfile" <<'PY'
import pathlib
import sys

manifest = pathlib.Path(sys.argv[1]).read_text()
gate_module = pathlib.Path(sys.argv[2]).read_text()
tauri_config = pathlib.Path(sys.argv[3]).read_text()
launch_controller = pathlib.Path(sys.argv[4])
release_audit = pathlib.Path(sys.argv[5])
release_audit_source = release_audit.read_text()
justfile = pathlib.Path(sys.argv[6]).read_text()

assert 'name = "atlas-desktop-gate"' in manifest
assert 'required-features = ["desktop-gate"]' in manifest
assert 'compile_error!' in gate_module
assert 'debug_assertions' in gate_module
assert 'insecure' not in manifest.lower()
assert '"frontendDist": "../../web/dist"' in tauri_config
assert launch_controller.is_file()
assert release_audit.is_file()
assert 'desktop-gate-release-audit:' in justfile
for required in ("b'rcgen'", "b'atlas_test_db'", "b'desktop-gate'"):
    assert required in release_audit_source
PY
