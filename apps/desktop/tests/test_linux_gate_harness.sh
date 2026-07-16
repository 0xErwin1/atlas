#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
gate="$repo_root/apps/desktop/tests/linux_gate.sh"
evidence=$(mktemp)
invalid_evidence=$(mktemp)
fixture_host=$(mktemp)
invocations=$(mktemp)

rm "$invalid_evidence"

cleanup() {
    rm -f "$evidence" "$invalid_evidence" "$fixture_host" "$invocations"
}
trap cleanup EXIT

cat >"$fixture_host" <<'HOST'
#!/usr/bin/env bash
set -euo pipefail

case_name=''
origin=''
while [ "$#" -gt 0 ]; do
    case "$1" in
        --atlas-linux-gate-case)
            case_name=${2:-}
            shift 2
            ;;
        --atlas-linux-gate-origin)
            origin=${2:-}
            shift 2
            ;;
        *)
            exit 64
            ;;
    esac
done

printf '%s %s\n' "$case_name" "$origin" >>"$ATLAS_GATE_INVOCATIONS"

case "$case_name" in
    login) result='{"outcome":"fail","failure_class":"authentication_failed","webkitgtk_version":"2.44.0"}' ;;
    protected_rest) result='{"outcome":"blocked","failure_class":"network_unreachable","webkitgtk_version":"2.44.0"}' ;;
    rust_bearer_sse) result='{"outcome":"fail","failure_class":"transport_unavailable","webkitgtk_version":"2.44.0"}' ;;
    restart_persistence) result='{"outcome":"blocked","failure_class":"session_unavailable","webkitgtk_version":"2.44.0"}' ;;
    expiry_or_revocation) result='{"outcome":"fail","failure_class":"session_invalid","webkitgtk_version":"2.44.0"}' ;;
    logout) result='{"outcome":"blocked","failure_class":"manual_validation_required","webkitgtk_version":"2.44.0"}' ;;
    remote_origin) result='{"outcome":"fail","failure_class":"remote_origin_rejected","webkitgtk_version":"2.44.0"}' ;;
    *) exit 65 ;;
esac

printf '%s\n' "$result"
HOST
chmod 700 "$fixture_host"

if ATLAS_GATE_INVOCATIONS="$invocations" bash "$gate" \
    --fixture \
    --binary "$fixture_host" \
    --origin https://atlas.example.test \
    --build-id test-build-1 \
    --evidence "$evidence"; then
    printf '%s\n' 'a non-passing fixture must keep the gate nonzero' >&2
    exit 1
fi

python3 - "$evidence" "$invocations" <<'PY'
import json
import pathlib
import sys

evidence = json.loads(pathlib.Path(sys.argv[1]).read_text())
invocations = pathlib.Path(sys.argv[2]).read_text().splitlines()
expected_cases = [
    "login",
    "protected_rest",
    "rust_bearer_sse",
    "restart_persistence",
    "expiry_or_revocation",
    "logout",
    "remote_origin",
]
expected_failures = {
    "login": "authentication_failed",
    "protected_rest": "network_unreachable",
    "rust_bearer_sse": "transport_unavailable",
    "restart_persistence": "session_unavailable",
    "expiry_or_revocation": "session_invalid",
    "logout": "manual_validation_required",
    "remote_origin": "remote_origin_rejected",
}

assert evidence["schema"] == "atlas.desktop.linux-gate-evidence/v1"
assert evidence["build"]["identifier"] == "test-build-1"
assert evidence["origin"] == "https://atlas.example.test"
assert evidence["timestamps"]["started_at"].endswith("Z")
assert evidence["timestamps"]["finished_at"].endswith("Z")
assert evidence["environment"]["os"] == "linux"
assert evidence["environment"]["architecture"] in {"x86_64", "aarch64"}
assert evidence["environment"]["runtime"] == "packaged-webkitgtk"
assert evidence["environment"]["webkitgtk_version"] == "2.44.0"
assert [case["name"] for case in evidence["cases"]] == expected_cases
assert {case["name"]: case["failure_class"] for case in evidence["cases"]} == expected_failures
assert len(invocations) == len(expected_cases)
assert [line.split(" ", 1)[0] for line in invocations] == expected_cases
assert all(line.endswith(" https://atlas.example.test") for line in invocations)
assert "test-build-1" not in json.dumps(evidence["cases"])
PY

for origin in \
    https://. \
    https://-bad.example \
    https://atlas..example.test \
    https://ATLAS.EXAMPLE.TEST \
    https://atlas.example.test:443 \
    http://atlas.example.test \
    https://user:password@atlas.example.test \
    https://atlas.example.test/path \
    'https://atlas.example.test?query=value' \
    'https://atlas.example.test#fragment' \
    'https://atlas.example.test ' \
    https://_bad.example; do
    if ATLAS_GATE_INVOCATIONS="$invocations" bash "$gate" \
        --fixture \
        --binary "$fixture_host" \
        --origin "$origin" \
        --build-id test-build-1 \
        --evidence "$invalid_evidence"; then
        printf 'noncanonical origin was accepted: %s\n' "$origin" >&2
        exit 1
    fi

    if [ -e "$invalid_evidence" ]; then
        printf '%s\n' 'invalid origins must not create evidence' >&2
        exit 1
    fi
done

for origin in \
    https://atlas.example.test \
    https://127.0.0.1 \
    'https://[2001:db8::1]'; do
    valid_evidence=$(mktemp)
    rm "$valid_evidence"

    if ATLAS_GATE_INVOCATIONS="$invocations" bash "$gate" \
        --fixture \
        --binary "$fixture_host" \
        --origin "$origin" \
        --build-id test-build-1 \
        --evidence "$valid_evidence"; then
        printf '%s\n' 'the fixture must retain a non-passing gate result' >&2
        exit 1
    fi

    python3 - "$valid_evidence" "$origin" <<'PY'
import json
import pathlib
import sys

evidence = json.loads(pathlib.Path(sys.argv[1]).read_text())
assert evidence["origin"] == sys.argv[2]
PY
    rm -f "$valid_evidence"
done
