# CLI

`atlas_cli` provides the `atlas` command. It is a thin, typed client over `atlas_client`.

Key implementation files:

- entry point: `crates/atlas_cli/src/main.rs`
- top-level parser: `crates/atlas_cli/src/cli.rs`
- command dispatch: `crates/atlas_cli/src/commands/`
- config resolution: `crates/atlas_cli/src/config.rs`
- output formatting: `crates/atlas_cli/src/output.rs`

## Global usage

```sh
atlas --help
atlas --base-url http://localhost:8080 --token "$ATLAS_TOKEN" workspaces list
atlas --workspace my-ws search "status:open tag:rust"
```

Global flags:

| Flag | Meaning |
|---|---|
| `--base-url` | Override the Atlas server URL. |
| `--token` | Bearer token to use for the request. |
| `--json` | Force JSON output. |
| `--workspace` | Default workspace slug for commands that need one. |

Workspace resolution order for workspace-scoped commands:

1. per-command `--workspace`
2. global `--workspace`

If neither is set, the CLI returns a validation error.

## Configuration, env vars, and keyring

Base URL precedence:

1. `--base-url`
2. `ATLAS_BASE_URL`
3. config file
4. `http://localhost:8080`

Token precedence:

1. `--token`
2. `ATLAS_TOKEN`
3. config file
4. OS keyring
5. none

Config path:

- `$XDG_CONFIG_HOME/atlas/config.toml`, or
- `$HOME/.config/atlas/config.toml`

Security details from `config.rs`:

- parent dir is created with mode `0700`
- config file is written with mode `0600`
- `config show` masks token output
- `config set-token` reads from stdin so the token does not have to appear in shell history

Useful commands:

```sh
atlas config path
atlas config show
atlas config set-url http://localhost:8080
printf '%s\n' "$ATLAS_TOKEN" | atlas config set-token --keyring
atlas config clear-token
```

## Output behavior

| Situation | Output mode |
|---|---|
| Interactive TTY, no `--json` | Human tables |
| `--json` passed | Pretty JSON |
| stdout is not a TTY | Pretty JSON |

List output uses the shared envelope:

```json
{
  "items": [],
  "next_cursor": null,
  "has_more": false
}
```

Human-mode pagination prints a follow-up cursor hint when more results are available.

## Command map

### Core browsing

| Command | Main subcommands |
|---|---|
| `version` | print CLI version |
| `search` | workspace search |
| `workspaces` | `list`, `get` |
| `projects` | `list`, `get` |
| `boards` | `list` |
| `columns` | `list` |
| `folders` | `list`, `get` |
| `tags` | `list` |
| `members` | `list` |
| `activity` | `list` |

### Documents

`atlas docs` supports:

- `list`
- `get`
- `create`
- `update-metadata`
- `update-content`
- `edit`
- `delete`
- `backlinks`
- `history`
- `revision`
- `frontmatter`
- `attach upload|list|download|delete`

Notable behavior:

- `docs update-content` is compare-and-swap and requires `--base-revision-id`.
- `docs edit` opens `$EDITOR`, then submits the content with the original revision id.
- if the post-editor update fails, the CLI preserves the edited text in a recovery file under the system temp dir (`atlas-edit-<slug>.md`).
- `docs create` and `docs update-metadata` support `--stdin` newline-delimited JSON batch mode.

Examples:

```sh
atlas docs get my-note --workspace my-ws --detail full
atlas docs update-content my-note --workspace my-ws --base-revision-id <uuid> --content-file ./note.md
atlas docs attach upload my-note ./image.png --workspace my-ws
```

### Tasks

`atlas tasks` supports:

- `list`, `get`, `create`, `update`, `move`, `delete`
- `refs list|create|remove`
- `backlinks`
- `assignees list|add|remove`
- `checklist list|add|update|remove|promote`
- `activity`
- `subtasks list|create|promote`
- `attach upload|list|download|delete`

Notable behavior:

- task IDs are readable IDs like `ATL-42`
- `tasks list` supports filters for board, status, assignee, repeated priorities, repeated labels, and sort
- `tasks create` and `tasks update` support `--stdin` newline-delimited JSON batch mode
- update arguments follow PATCH semantics; clearable fields have explicit clear flags in single-item mode and accept `null` in batch mode
- board names can resolve by case-insensitive substring; mutation paths still require exactly one resolved target

Examples:

```sh
atlas tasks list --workspace my-ws --board product --status todo --priority high
atlas tasks create --workspace my-ws --board product --column Todo --title "Write docs"
atlas tasks update ATL-42 --workspace my-ws --title "Reword docs" --clear-due-date
atlas tasks refs create ATL-42 --workspace my-ws --kind relates --target ATL-10
```

### Workspace settings and sharing

| Command | Main subcommands |
|---|---|
| `grants workspace` | `list`, `create`, `revoke` |
| `grants project` | `list`, `create`, `revoke` |
| `groups` | `list`, `create`, `delete`, `add-member`, `remove-member`, `members` |
| `status-templates` | `list`, `create`, `update`, `delete`, `apply` |
| `saved-searches` | `list`, `create`, `rename`, `delete` |
| `task-views` | `list`, `get`, `create`, `update`, `delete` |
| `property-definitions` | `list`, `create`, `delete` |
| `api-keys` | `list`, `create`, `revoke`, `set-global`, `grants`, `delete-grant` |
| `audit` | `workspace`, `platform` |

### Admin commands

| Command | Main subcommands |
|---|---|
| `users` | `list`, `create`, `disable`, `enable`, `reset-password`, `regenerate-link`, `memberships` |

User creation creates a pending account and returns an activation link. The CLI does not expose the REST `system-admin` toggle route.

### Shell ergonomics and config

| Command | Purpose |
|---|---|
| `completions` | generate shell completions for bash/elvish/fish/powershell/zsh |
| `config` | manage config path/url/token state |

### Import/export

| Command | Purpose |
|---|---|
| `import obsidian` | import an Obsidian vault into an existing Atlas project |
| `export obsidian` | export an Atlas project as an Obsidian-style vault |

Import details backed by `commands/import/obsidian`:

- requires an existing target project
- supports `--dry-run`
- prompts unless `--yes` is passed
- maintains a `.atlas-import.json` manifest for resumable imports
- executes folders, documents, boards/tasks, then attachments

Export details backed by `commands/export/obsidian`:

- requires an existing source project
- supports `--dry-run`
- exports folders, documents, and board/task files
- skips standalone export files for tasks already backed by a `docs` reference

## Confirmations and destructive actions

Many destructive commands require an explicit confirmation flag, for example:

- `--confirm` on deletes, revocations, and some security-sensitive mutations
- `--yes` on Obsidian import

This is enforced by clap parsing or command validation, not by convention alone.

## Current gaps and quirks

- there is no `atlas login` command
- some REST endpoints are not wrapped by CLI commands, especially activation, webhooks, integration configs, automation rules, and the root-only system-admin toggle
- board/column name resolution is convenient but can be ambiguous
- JSON is automatic when stdout is piped
