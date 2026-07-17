# Contributing

All documentation and code is in English.

## Commit workflow

Commits land directly on `main`. There is no PR workflow for this private repo.

Commit identity:

```
Ignacio Perez <ignacio@feuer.me>
```

Run the local gate before committing:

```sh
check && tests
```

For a full pre-commit sweep:

```sh
verify
```

## Commit message format

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <short description>
```

Allowed types: `feat`, `fix`, `chore`, `refactor`, `docs`, `test`, `ci`.

Examples:

```
feat(atlas_server): add pagination to resource list
fix(atlas_client): retry on transient network errors
chore: update Rust toolchain to 1.97
```

## Adding a crate

1. Create `crates/<name>/` with `Cargo.toml` and `src/lib.rs`.
2. Add it to `members` (and `default-members` if it is a product crate) in the root `Cargo.toml`.
3. Run `cargo check --workspace` before committing.

## Keeping toolchain pins in sync

The Rust version appears in two places:

- `flake.nix` — `pkgs.rust-bin.stable."X.Y.Z".default`
- `.github/workflows/style.yml` and `tests.yml` — `dtolnay/rust-toolchain@stable` with `toolchain: 'X.Y'`

Update both when bumping the toolchain.
