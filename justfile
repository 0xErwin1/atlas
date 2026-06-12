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

test:
    cargo nextest run --workspace
    cargo test --doc --workspace

build:
    cargo build --workspace

db-up:
    podman compose up -d postgres

db-down:
    podman compose down

migrate:
    cargo run -p migration -- up

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
