#!/usr/bin/env bash
set -euo pipefail

root=$(git rev-parse --show-toplevel)
controller="$root/apps/desktop/gate/run_webdriver_launch.sh"
flake="$root/flake.nix"

python3 - "$controller" "$flake" <<'PY'
import pathlib
import sys

controller = pathlib.Path(sys.argv[1])
flake = pathlib.Path(sys.argv[2]).read_text()
assert controller.is_file()
assert 'desktop-gate.exec' in flake
assert 'launch)' in flake

source = controller.read_text()
for required in (
    'pnpm --filter @atlas/web build',
    'Xvfb',
    'WebKitWebDriver',
    'tauri-driver',
    'tauri:options',
    'ATLAS_DESKTOP_GATE_EVIDENCE_PATH',
):
    assert required in source

for prohibited in ('screenshot', 'page source', 'capture-log', 'print-page-source'):
    assert prohibited not in source.lower()
PY
