#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
secret='fixture-bearer-must-not-appear'
output=$(mktemp)

cleanup() {
    rm -f "$output"
}
trap cleanup EXIT

if ! cargo build --quiet -p atlas_desktop --bin atlas-desktop-gate --features desktop-gate; then
    printf '%s\n' 'the non-shipping desktop gate binary must build with its required feature' >&2
    exit 1
fi

if ! cargo nextest run -p atlas_desktop --features desktop-gate \
    --test gate_controller controller_keeps_login_private_and_tears_down_its_ephemeral_resources; then
    printf '%s\n' 'the required-feature gate controller protocol must pass' >&2
    exit 1
fi

if cargo run --quiet -p atlas_desktop -- \
    --atlas-linux-gate-case login \
    --atlas-linux-gate-origin https://atlas.example.test \
    --token "$secret" >"$output" 2>&1; then
    printf '%s\n' 'the desktop host must reject token-like gate arguments' >&2
    exit 1
fi

if grep -Fq "$secret" "$output"; then
    printf '%s\n' 'the desktop host echoed bearer material' >&2
    exit 1
fi
