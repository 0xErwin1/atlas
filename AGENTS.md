# AGENTS.md — Atlas

Entry point for AI coding agents (Claude Code, Cursor, and any AGENTS.md-aware tool). The **canonical, detailed agent guide is [CLAUDE.md](CLAUDE.md)** — read it first. For system structure (crates, request lifecycle, data model, permissions, web frontend), see [ARCHITECTURE.md](ARCHITECTURE.md).

## Non-negotiables

The full guide is in CLAUDE.md; these are the rules that break the build or the repo if missed:

- **Dev shell.** NixOS host with no system toolchain — run every `cargo`/`just`/`pnpm` command via `nix develop --command <cmd>`. Containers are **podman**, not docker.
- **Gate before committing.** `just verify` (fmt-check + clippy `-D warnings` + nextest + build + web lint) must be green.
- **Strict TDD.** Failing test first, then implement. No `panic`/`unwrap`/`expect` outside tests; `unsafe` is forbidden; `atlas_domain` stays pure (no axum/sea-orm/tokio).
- **Commits.** Conventional Commits straight to `main`, one atomic work-unit each (code + its test), author `Ignacio Perez <ignacio@feuer.me>`, no co-author trailers. English for all code, comments, docs, and commit messages.
- **Contract sync.** After any backend API change, regenerate the typed web client with `just gen-types` (never hand-edit `apps/web/src/api/types.d.ts`).

See [CONTRIBUTING.md](CONTRIBUTING.md) for the commit workflow and [CODE_STYLE.md](CODE_STYLE.md) for lint policy.
