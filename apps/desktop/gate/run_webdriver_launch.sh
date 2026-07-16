#!/usr/bin/env bash
set -euo pipefail

root=$(git rev-parse --show-toplevel)
work_dir=$(mktemp -d)
display_file="$work_dir/display"
ca_path="$work_dir/ca.pem"
key_path="$work_dir/ca-key.pem"
evidence_path="$work_dir/evidence"

cleanup() {
    for pid in "${session_pid:-}" "${driver_pid:-}" "${xvfb_pid:-}"; do
        if [ -n "$pid" ]; then
            kill "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
        fi
    done
    rm -rf "$work_dir"
}
trap cleanup EXIT

port() {
    python3 - <<'PY'
import socket

with socket.socket() as listener:
    listener.bind(("127.0.0.1", 0))
    print(listener.getsockname()[1])
PY
}

wait_for_status() {
    local endpoint=$1
    for _ in $(seq 1 100); do
        if curl --silent --fail "$endpoint" >/dev/null; then
            return 0
        fi
        sleep 0.1
    done
    return 1
}

cd "$root"
pnpm --filter @atlas/web build >/dev/null
cargo build --quiet -p atlas_desktop --bin atlas-desktop-gate --features desktop-gate

openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=atlas-desktop-gate \
    -keyout "$key_path" -out "$ca_path" >/dev/null 2>&1

Xvfb -displayfd 1 -screen 0 1200x800x24 -nolisten tcp >"$display_file" 2>/dev/null &
xvfb_pid=$!
for _ in $(seq 1 100); do
    if [ -s "$display_file" ]; then
        break
    fi
    sleep 0.1
done
display=$(tr -d '[:space:]' <"$display_file")
test -n "$display"

driver_port=$(port)
native_port=$(port)
DISPLAY=":$display" \
ATLAS_DESKTOP_ORIGIN=https://localhost:1 \
ATLAS_DESKTOP_GATE_CA_PATH="$ca_path" \
ATLAS_DESKTOP_GATE_EVIDENCE_PATH="$evidence_path" \
tauri-driver --port "$driver_port" --native-port "$native_port" \
    --native-driver "$(command -v WebKitWebDriver)" >/dev/null 2>&1 &
driver_pid=$!
wait_for_status "http://127.0.0.1:$driver_port/status"

session_response=$(curl --silent --fail \
    -H 'Content-Type: application/json' \
    -d "{\"capabilities\":{\"alwaysMatch\":{\"browserName\":\"wry\",\"tauri:options\":{\"application\":\"$root/target/debug/atlas-desktop-gate\"}}}}" \
    "http://127.0.0.1:$driver_port/session")
session_id=$(python3 -c 'import json, sys; print(json.load(sys.stdin)["value"]["sessionId"])' <<<"$session_response")

ipc_response=$(curl --silent --fail \
    -H 'Content-Type: application/json' \
    -d '{"script":"const done = arguments[0]; window.__TAURI_INTERNALS__.invoke(\u0027desktop_session_status\u0027).then(() => done(\u0027ok\u0027)).catch(() => done(\u0027command-rejected\u0027));","args":[]}' \
    "http://127.0.0.1:$driver_port/session/$session_id/execute/async")
python3 -c 'import json, sys; assert json.load(sys.stdin)["value"] == "command-rejected"' <<<"$ipc_response"

curl --silent --fail -X DELETE "http://127.0.0.1:$driver_port/session/$session_id" >/dev/null
session_pid=''

python3 - "$evidence_path" <<'PY'
import pathlib
import sys

assert pathlib.Path(sys.argv[1]).read_text() == "phase=webdriver-launch outcome=started\n"
PY
cat "$evidence_path"
