export DATABASE_URL := env_var_or_default("DATABASE_URL", "postgres://atlas:atlas@localhost:5432/atlas_dev")

default:
    @just --list

check:
    cargo check --workspace

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

test: db-wait
    cargo nextest run --workspace
    cargo test --doc --workspace

build:
    cargo build --workspace

db-up:
    podman compose up -d postgres

db-down:
    podman compose down

db-wait:
    #!/usr/bin/env bash
    set -e
    echo "Waiting for Postgres..."
    for i in $(seq 1 30); do
        if podman compose exec postgres pg_isready -U atlas -d atlas_dev -q 2>/dev/null; then
            echo "Postgres ready."
            exit 0
        fi
        sleep 1
    done
    echo "Postgres did not become ready in time." >&2
    exit 1

db-reset:
    cargo run -p migration -- fresh

db-clean-tests:
    #!/usr/bin/env bash
    set -e
    psql "$DATABASE_URL" -tc "SELECT datname FROM pg_database WHERE datname LIKE 'atlas_test_%'" \
        | while IFS= read -r db; do
            db=$(echo "$db" | tr -d '[:space:]')
            if [ -n "$db" ]; then
                echo "Dropping test database: $db"
                psql "$DATABASE_URL" -c "DROP DATABASE IF EXISTS \"$db\" WITH (FORCE)"
            fi
        done

migrate:
    cargo run -p migration -- up

seed-dev:
    cargo run -p atlas_server --bin seed_dev

gen-types:
    cargo run -p atlas_server --bin dump_openapi > apps/web/openapi.json
    pnpm --filter @atlas/web exec openapi-typescript openapi.json -o src/api/types.d.ts

dev-web: gen-types
    pnpm --filter @atlas/web dev

build-web: gen-types
    pnpm --filter @atlas/web build

lint-web:
    pnpm exec biome ci .

desktop-dev:
    cd apps/desktop/src-tauri && cargo tauri dev

desktop-gate-red:
    bash apps/desktop/tests/test_linux_gate_harness.sh

desktop-gate:
    bash apps/desktop/tests/linux_gate.sh --evidence "${ATLAS_DESKTOP_GATE_EVIDENCE_PATH:-/tmp/atlas-desktop-gate-evidence.json}"

desktop-gate-asset-audit:
    bash apps/desktop/tests/linux_gate.sh --asset-audit

desktop-gate-tooling:
    bash apps/desktop/gate/test_tauri_driver_tooling.sh

desktop-gate-release-audit:
    bash apps/desktop/gate/audit_release_exclusion.sh

desktop-gate-launch:
    bash apps/desktop/gate/run_webdriver_launch.sh

desktop-gate-controller-test:
    VITE_ATLAS_DESKTOP_GATE=1 pnpm --filter @atlas/web build
    cargo build -p atlas_desktop --features desktop-gate --bin atlas-desktop-gate
    cargo nextest run -p atlas_desktop --features desktop-gate --test gate_controller

desktop-gate-webdriver-test:
    VITE_ATLAS_DESKTOP_GATE=1 pnpm --filter @atlas/web build
    cargo build -p atlas_desktop --features desktop-gate --bin atlas-desktop-gate
    cargo nextest run -p atlas_desktop --features desktop-gate --test gate_controller controller_drives_the_packaged_vue_webdriver_login_and_restart_flow

desktop-host-test:
    bash apps/desktop/tests/test_desktop_host.sh

fmt-web:
    pnpm exec biome format --write .

dev: db-up
    cargo run -p atlas_server

# Bring up API (:8080) + web (:5173) without managing Postgres.
up-no-db:
    #!/usr/bin/env bash
    set -euo pipefail
    export ATLAS_ROOT_PASSWORD="${ATLAS_ROOT_PASSWORD:-rootdev}"
    export PC_PORT_NUM="${PC_PORT_NUM:-8079}"
    process-compose -f process-compose.no-db.yaml up

# Bring up the whole stack for manual testing with process-compose:
# Postgres + dev seed (root user & sample workspace) + API (:8080) + web (:5173),
# ordered by dependencies with health checks (see process-compose.yaml).
# Login: user `root`, password = $ATLAS_ROOT_PASSWORD (default `rootdev`).
# Quit the TUI with `q` / Ctrl-C — process-compose tears everything down.
up:
    #!/usr/bin/env bash
    set -euo pipefail
    export ATLAS_ROOT_PASSWORD="${ATLAS_ROOT_PASSWORD:-rootdev}"
    # process-compose's own REST API defaults to :8080 and would clash with the
    # Atlas API; move it out of the way via its native env var.
    export PC_PORT_NUM="${PC_PORT_NUM:-8079}"
    process-compose -f process-compose.yaml up

# Build the two deploy OCI images from the deploy/ Containerfiles (repo-root
# context, honors .dockerignore). atlas-server holds both the atlas_server and
# atlas_mcp binaries; atlas-web serves the SPA and reverse-proxies /api and /mcp.
# The server deploys these via Ansible; this just builds them (e.g. before
# `deploy/deploy.sh` ships them over the VPN). Override the tag with
# e.g. `just build-images latest`.
build-images tag="local":
    podman build -t atlas-server:{{tag}} -f deploy/Containerfile.server .
    podman build -t atlas-web:{{tag}} -f deploy/Containerfile.web .

verify: fmt-check clippy test build lint-web build-web
