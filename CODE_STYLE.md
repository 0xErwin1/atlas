# Code Style

Atlas favors strict, boring, maintainable code, small focused changes, and behavior-preserving refactors. Follow the existing crate and package conventions before introducing new patterns. This file covers coding conventions and verification expectations; `AGENTS.md` holds agent behavior rules and `ARCHITECTURE.md` the system structure.

## Rust

### Naming

| Element | Convention | Examples |
|---------|------------|----------|
| Types (struct/enum/trait) | `PascalCase` | `WorkspaceCtx`, `DocumentRepo` |
| Functions / methods | `snake_case` | `list_documents`, `apply_diff` |
| Fields / locals | `snake_case` | `workspace_id`, `parent_task_id` |
| Constants / statics | `UPPER_SNAKE_CASE` | `ROUTE_REGISTRY` |
| Tests | `snake_case` (often `test_` prefix) | `test_promote_clears_parent` |

Use full words for identifiers; avoid abbreviations (`queue`, not `q`).

### File organization

- The canonical crate map and key-file overview live in `ARCHITECTURE.md`. Keep new code in the crate and area that already owns the behavior instead of creating parallel structure.
- Module directories use `mod.rs` (e.g. `services/mod.rs`, `authz/mod.rs`), not a sibling `services.rs`.
- Prefer implementing in existing files unless the change is a genuinely new logical component. Avoid creating many small files.
- Respect the compiler-enforced dependency direction: `atlas_domain` stays pure (no axum/sea-orm/tokio); SeaORM types never leak out of `atlas_server/src/persistence/`.

### Imports

- Group imports at the top; use braces for multi-item paths.
- Prefer `crate::` and `super::` for local modules; keep external dependencies grouped separately from internal modules.

### Lint policy

Workspace-level lints in the root `Cargo.toml` apply to every crate:

| Lint | Level | Notes |
|------|-------|-------|
| `rust::unsafe_code` | `forbid` | No `unsafe` blocks anywhere; cannot be locally overridden. |
| `rust::unused_must_use` | `deny` | Dropping a `Result` silently is a bug. |
| `clippy::unwrap_used` | `deny` | Use `?` or `anyhow::Context` instead. |
| `clippy::expect_used` | `deny` | Same. |
| `clippy::panic` | `deny` | Same. |
| `clippy::unwrap_in_result` | `deny` | Same. |
| `clippy::dbg_macro` | `deny` | No `dbg!` reaches `main`. |
| `clippy::todo` | `warn` | Promoted to error by CI `-D warnings`. |
| `clippy::unimplemented` | `warn` | Same. |
| `clippy::indexing_slicing` | `warn` | Same. |

All crates must include `[lints] workspace = true` in their `Cargo.toml`.

### Test escape hatch

Test code may use `unwrap`/`expect`/`panic`. Each crate's root carries:

```rust
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
```

### Toolchain pins

The Rust version is pinned in two places and must stay in sync:

- `flake.nix` — `pkgs.rust-bin.stable."1.96.0".default`
- CI workflows (`style.yml`, `tests.yml`) — `toolchain: '1.96'`

When bumping, update both files in the same commit.

### Error handling

- Domain and request errors are typed (`thiserror` in `atlas_domain`; RFC 9457 problem responses in `atlas_server`). Return `Result`; propagate with `?`.
- **Never silently discard a fallible expression with `let _ =`.** The `unused_must_use` deny lint already rejects a dropped `Result`; do not work around it. Propagate with `?`, branch with `match`/`if let`, log it, or surface it to the user.
- Prefer `get`/`get_mut` over indexing that can panic on out-of-bounds, and handle the `None` arm (`indexing_slicing` warns).

### Logging

- Use the `tracing` crate (`tracing::{info, warn, error, debug}`); the subscriber is initialized at the binary entry point.
- Never log secrets: `ATLAS_ROOT_PASSWORD`, session tokens, and API-key values must never be logged or printed.

### Comments

Default to **no inline comments**. Add one only when the reason is non-obvious: a hidden constraint, a workaround for a specific bug, or behavior that would surprise a reader. Function-level doc comments (`///`) for intent, invariants, and side effects are preferred over inline statement comments. Never write comments that restate what the code does; never reference a task, PR, or ticket.

### Function size

Treat a function growing beyond ~100 lines as a design smell. Extract well-named helpers that preserve behavior exactly; keep such refactors local (no public-API or cross-module moves unless requested).

## TypeScript / Vue

All TypeScript and Vue code is formatted and linted with [Biome](https://biomejs.dev/) (`biome.json` at the repo root).

Key settings:

- Indent: 2 spaces
- Line width: 110
- Quotes: single
- Trailing commas: all
- Semicolons: always
- Recommended rules enabled

Run locally:

```sh
just lint-web    # check
just fmt-web     # auto-fix
```

No ESLint or Prettier — Biome replaces both.

### Conventions

- Keep `strict` mode compatibility. Prefer explicit domain types over broad objects; avoid `any`. If a cast is unavoidable, keep it local and make the invariant clear through a narrow helper.
- Handle `null` and `undefined` explicitly instead of relying on truthiness when values can be `0`, `false`, or an empty string.
- Keep frontend imports browser-safe; it speaks the REST contract only and never touches the DB.
- Match existing component and Pinia store patterns. Prefer readable Vue templates over dense inline logic.

### Patterns

- **Reuse, don't reimplement — no duplication (non-negotiable).** Before writing UI, use the shared primitive; never copy its markup, styles, or behavior into a panel. Canonical primitives: single-select dropdown → `Dropdown` (`src/components/ui/Dropdown.vue`); anchored menu / popover surface → `Popover`; confirmation → `ConfirmDialog`; form field + validation → `FormField` + `validateForm`; expandable settings row (collapsed summary + inline manage panel, whole-row click) → `ExpandableRow` (`src/components/settings/ExpandableRow.vue`). The moment a visual or behavioral pattern appears a *second* time, extract one component and have every call site use it. Duplicated markup or CSS across components is a defect to remove, not to extend — this includes "same thing styled by copy-pasted classes". When you reach for a `<select>`, a hand-built menu, a custom toggle, or a re-styled row, stop and use (or extract) the shared component instead.
- **Generated API client.** `apps/web/src/api/types.d.ts` is generated from the served OpenAPI by `just gen-types`. Never hand-edit it; regenerate after a backend contract change.
- **Form validation.** Validate with [zod](https://zod.dev/) through the shared `FormField` (`src/components/ui/FormField.vue`) and `validateForm` (`src/lib/validation.ts`). Show the API problem `hint` on failure; do not rely on native browser validation bubbles.
- **Comments.** Same rule as Rust: default to none; explain only a non-obvious *why*.

## Testing expectations

- **Strict TDD.** Write the failing test first, see it red, then implement to green.
- Rust unit tests live beside the implementation in `#[cfg(test)] mod tests` blocks; run with `cargo nextest`. Doctests run separately.
- Integration tests need Postgres running (`just db-up`); the harness creates and drops one database per test. Cross-tenant isolation is covered by integration tests — preserve it.
- Web tests use Vitest and `vue-tsc`.

| Change type | Expected verification |
|-------------|----------------------|
| Backend behavior (domain, services, repos, routes) | `cargo nextest` tests in the affected crate |
| Backend API/contract change | regenerate `types.d.ts` (`just gen-types`); the OpenAPI zero-drift test must stay green |
| Frontend logic, forms, or composables | Vitest tests under `apps/web/src` |
| Anything before pushing | `just verify` (fmt-check + clippy + test + build + web lint) green |

## Documentation conventions

- Keep repository documentation factual and current with the codebase. Prefer tables, short checklists, and direct examples over long prose.
- Update `README.md`, `ARCHITECTURE.md`, `AGENTS.md`, or this file when commands, boundaries, or conventions change.
- Do not add personal working-note markdown files to the repository.
