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

dev-web:
    pnpm --filter @atlas/web dev

build-web:
    pnpm --filter @atlas/web build

lint-web:
    pnpm exec biome ci .

fmt-web:
    pnpm exec biome format --write .

dev: db-up
    cargo run -p atlas_server

verify: fmt-check clippy test build lint-web
