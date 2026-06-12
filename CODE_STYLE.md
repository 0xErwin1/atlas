# Code Style

## Rust

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

### Comments

Default to **no inline comments**. Add one only when the reason is non-obvious: a hidden constraint, a workaround for a specific bug, or behavior that would surprise a reader. Function-level doc comments (`///`) for intent, invariants, and side effects are preferred over inline statement comments. Never write comments that restate what the code does.

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
