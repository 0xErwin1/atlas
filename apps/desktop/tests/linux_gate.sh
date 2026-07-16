#!/usr/bin/env bash
set -euo pipefail

build_gate_binary() {
    VITE_ATLAS_DESKTOP_GATE=1 pnpm --filter @atlas/web build >/dev/null
    assert_gate_observer_asset
    cargo build --quiet -p atlas_desktop --features desktop-gate \
        --bin atlas-desktop-gate --bin atlas-desktop-gate-controller
}

assert_gate_observer_asset() {
    grep -aRFq '__atlasDesktopGateLiveUpdateObservation' apps/web/dist
}

if [ "${1:-}" = "--asset-audit" ]; then
    repo_root=$(git rev-parse --show-toplevel)
    cd "$repo_root"

    build_gate_binary

    pnpm --filter @atlas/web build >/dev/null
    ! grep -aRFq '__atlasDesktopGateLiveUpdateObservation' apps/web/dist
    cargo build --quiet -p atlas_desktop --bin atlas-desktop
    exit 0
fi

if [ "${1:-}" != "--fixture" ]; then
    repo_root=$(git rev-parse --show-toplevel)
    evidence=''

    if [ "${1:-}" = "--evidence" ] && [ -n "${2:-}" ] && [ "$#" -eq 2 ]; then
        evidence=$2
    else
        printf '%s\n' 'usage: linux_gate.sh --evidence PATH | --asset-audit | --fixture --binary PATH --origin HTTPS_ORIGIN --build-id ID --evidence PATH' >&2
        exit 2
    fi

    cd "$repo_root"
    build_gate_binary
    pkill -f -- "$repo_root/target/debug/atlas-desktop-gate" 2>/dev/null || true
    "$repo_root/target/debug/atlas-desktop-gate-controller" \
        --application "$repo_root/target/debug/atlas-desktop-gate" \
        --evidence "$evidence"
    exit 0
fi

shift

binary=''
origin=''
build_id=''
evidence=''

usage() {
    printf '%s\n' 'usage: linux_gate.sh --binary PATH --origin HTTPS_ORIGIN --build-id ID --evidence PATH' >&2
    exit 2
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --binary)
            binary=${2:-}
            shift 2
            ;;
        --origin)
            origin=${2:-}
            shift 2
            ;;
        --build-id)
            build_id=${2:-}
            shift 2
            ;;
        --evidence)
            evidence=${2:-}
            shift 2
            ;;
        *)
            usage
            ;;
    esac
done

if [ -z "$binary" ] || [ -z "$origin" ] || [ -z "$build_id" ] || [ -z "$evidence" ]; then
    usage
fi

python3 - "$binary" "$origin" "$build_id" "$evidence" <<'PY'
import datetime
import ipaddress
import json
import os
import re
import subprocess
import sys
import urllib.parse

CASES = (
    "login",
    "protected_rest",
    "rust_bearer_sse",
    "restart_persistence",
    "expiry_or_revocation",
    "logout",
    "remote_origin",
)
OUTCOMES = {"pass", "fail", "blocked"}
FAILURE_CLASSES = {
    "none",
    "authentication_failed",
    "host_execution_failed",
    "manual_validation_required",
    "network_unreachable",
    "packaged_host_unavailable",
    "protocol_invalid",
    "remote_origin_rejected",
    "session_invalid",
    "session_unavailable",
    "transport_unavailable",
}
BUILD_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}\Z")
DNS_LABEL = re.compile(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\Z")
WEBKITGTK_VERSION = re.compile(r"[0-9][0-9A-Za-z._+-]{0,63}\Z")


def timestamp() -> str:
    return datetime.datetime.now(datetime.UTC).isoformat(timespec="seconds").replace("+00:00", "Z")


def canonical_origin(value: str) -> str:
    if value != value.strip() or value != value.lower():
        raise ValueError("origin is not canonical")

    parsed = urllib.parse.urlsplit(value)
    if parsed.scheme != "https" or not parsed.netloc:
        raise ValueError("origin must use HTTPS")
    if parsed.username is not None or parsed.password is not None:
        raise ValueError("origin contains userinfo")
    if parsed.path or parsed.query or parsed.fragment:
        raise ValueError("origin contains unsupported components")

    try:
        port = parsed.port
    except ValueError as error:
        raise ValueError("origin port is invalid") from error

    if port == 443:
        raise ValueError("origin contains the default HTTPS port")

    host = parsed.hostname
    if host is None:
        raise ValueError("origin host is missing")

    if parsed.netloc.startswith("["):
        if not parsed.netloc.startswith(f"[{host}]"):
            raise ValueError("origin IPv6 host is not canonical")
        try:
            address = ipaddress.IPv6Address(host)
        except ipaddress.AddressValueError as error:
            raise ValueError("origin host is unsupported") from error
        if host != address.compressed:
            raise ValueError("origin IPv6 host is not canonical")
        return value

    if ":" in host:
        raise ValueError("origin host is unsupported")

    try:
        address = ipaddress.IPv4Address(host)
    except ipaddress.AddressValueError:
        labels = host.split(".")
        if len(host) > 253 or not labels or any(not DNS_LABEL.fullmatch(label) for label in labels):
            raise ValueError("origin host is unsupported")
    else:
        if host != str(address):
            raise ValueError("origin IPv4 host is not canonical")

    return value


def result_for(binary: str, origin: str, case: str) -> tuple[dict[str, str], str]:
    try:
        completed = subprocess.run(
            [binary, "--atlas-linux-gate-case", case, "--atlas-linux-gate-origin", origin],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=30,
        )
    except FileNotFoundError:
        return {"outcome": "blocked", "failure_class": "packaged_host_unavailable"}, "unavailable"
    except (OSError, subprocess.TimeoutExpired):
        return {"outcome": "blocked", "failure_class": "host_execution_failed"}, "unavailable"

    try:
        result = json.loads(completed.stdout)
        outcome = result["outcome"]
        failure_class = result["failure_class"]
        webkitgtk_version = result["webkitgtk_version"]
    except (json.JSONDecodeError, KeyError, TypeError):
        return {"outcome": "blocked", "failure_class": "protocol_invalid"}, "unavailable"

    if (
        completed.returncode != 0
        or outcome not in OUTCOMES
        or failure_class not in FAILURE_CLASSES
        or not isinstance(webkitgtk_version, str)
        or not WEBKITGTK_VERSION.fullmatch(webkitgtk_version)
    ):
        return {"outcome": "blocked", "failure_class": "protocol_invalid"}, "unavailable"

    return {"outcome": outcome, "failure_class": failure_class}, webkitgtk_version


binary, origin, build_id, evidence_path = sys.argv[1:]

if not BUILD_ID.fullmatch(build_id):
    print("build identifier is invalid", file=sys.stderr)
    sys.exit(2)

architecture = os.uname().machine
if architecture not in {"x86_64", "aarch64"}:
    print("architecture is unsupported", file=sys.stderr)
    sys.exit(2)

try:
    origin = canonical_origin(origin)
except ValueError:
    print("origin must be a canonical credential-free HTTPS origin", file=sys.stderr)
    sys.exit(2)

started_at = timestamp()
cases = []
webkitgtk_versions = set()
for case in CASES:
    result, webkitgtk_version = result_for(binary, origin, case)
    cases.append({"name": case, **result})
    if webkitgtk_version != "unavailable":
        webkitgtk_versions.add(webkitgtk_version)

finished_at = timestamp()
observed_webkitgtk = next(iter(webkitgtk_versions)) if len(webkitgtk_versions) == 1 else "unavailable"
evidence = {
    "schema": "atlas.desktop.linux-gate-evidence/v1",
    "build": {"identifier": build_id},
    "timestamps": {"started_at": started_at, "finished_at": finished_at},
    "environment": {
        "os": "linux",
        "architecture": architecture,
        "runtime": "packaged-webkitgtk",
        "webkitgtk_version": observed_webkitgtk,
    },
    "origin": origin,
    "cases": cases,
}

os.makedirs(os.path.dirname(evidence_path) or ".", exist_ok=True)
descriptor = os.open(evidence_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
with os.fdopen(descriptor, "w", encoding="utf-8") as evidence_file:
    json.dump(evidence, evidence_file, separators=(",", ":"), sort_keys=True)
    evidence_file.write("\n")

if all(case["outcome"] == "pass" for case in cases):
    sys.exit(0)

print("the blocking Linux gate did not pass all required cases", file=sys.stderr)
sys.exit(1)
PY
