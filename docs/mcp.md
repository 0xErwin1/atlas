# MCP

`atlas_mcp` exposes Atlas through the Model Context Protocol using the same `atlas_client` and REST API as the CLI.

Key implementation files:

- server implementation: `crates/atlas_mcp/src/lib.rs`
- binary entry point and transport setup: `crates/atlas_mcp/src/main.rs`

## Transports

```sh
# default: stdio
ATLAS_BASE_URL=http://localhost:8080 ATLAS_TOKEN=atlas_... atlas_mcp

# Streamable HTTP on /mcp
ATLAS_BASE_URL=http://localhost:8080 atlas_mcp --transport http --bind 127.0.0.1 --port 3001
```

| Setting | Default | Meaning |
|---|---|---|
| `--transport`, `ATLAS_MCP_TRANSPORT` | `stdio` | `stdio` or `http` |
| `--bind`, `ATLAS_MCP_BIND` | `127.0.0.1` | HTTP bind address |
| `--port`, `ATLAS_MCP_PORT` | `3001` | HTTP port |
| `ATLAS_BASE_URL` | `http://localhost:8080` | Atlas REST API base URL |
| `ATLAS_TOKEN` | required in stdio mode | startup bearer token |

HTTP mode mounts the MCP service at `/mcp`.

## Authentication model

### Stdio mode

- requires `ATLAS_TOKEN` at startup
- stores one startup token for all tool calls
- does a best-effort `/v1/auth/me` identity probe on startup
- a failed identity probe logs a warning but does not abort startup

### HTTP mode

- stores no startup token
- every MCP request must include `Authorization: Bearer atlas_<token>`
- invalid or missing bearer headers are rejected at the HTTP middleware layer with `401`

The code explicitly prefers API keys for agent attribution. If a token authenticates as a human user instead, the server logs that attribution will be user-based rather than agent-based.

## Advertised capabilities

`atlas_mcp` advertises:

- tools
- resources

It does **not** advertise prompts.

## Resource support

Resource template:

```text
atlas:///{workspace}/{slug}
```

Behavior backed by `read_resource` and URI parsing helpers:

- `workspace` is a workspace slug
- `slug` is a document slug or UUID
- only this one resource template is advertised
- `read_resource` returns the document body as `text/markdown`
- malformed schemes, missing segments, and extra path segments are rejected

## Tool conventions

Shared behavior from `ATLAS_INSTRUCTIONS` and tool parameter docs:

- discover before mutating
- list calls return `{items, next_cursor, has_more}`
- heavy reads are compact by default; use `detail=full` where supported
- PATCH-style tools distinguish omitted fields from explicit `null`
- destructive tools require `confirm: true`
- document content edits are CAS-based and return structured conflict data
- some write tools resolve boards/columns by name and return actionable errors listing valid options on misses

## Tool catalog

### Discovery and read tools

- `ping`
- `search`
- `get_document`
- `list_tasks`
- `get_task`
- `list_documents`
- `list_folders`
- `list_boards`
- `list_columns`
- `list_tags`
- `list_used_labels`
- `list_members`
- `list_workspaces`
- `list_projects`
- `list_saved_searches`
- `list_task_views`
- `get_task_references`
- `get_task_backlinks`
- `get_document_backlinks`
- `list_checklist`
- `list_activity`
- `list_workspace_activity`
- `list_document_history`
- `get_document_revision`
- `list_attachments`
- `get_workspace_audit`
- `get_platform_audit`

### Task write tools

- `create_task`
- `update_task`
- `move_task`
- `delete_task`
- `add_task_assignee`
- `remove_task_assignee`
- `add_task_reference`
- `remove_task_reference`
- `add_checklist_item`
- `update_checklist_item`
- `delete_checklist_item`
- `promote_checklist_item`
- `create_subtask`
- `promote_subtask`

### Document and folder write tools

- `create_document`
- `update_document_metadata`
- `update_document_content`
- `delete_document`
- `move_document`
- `copy_document`
- `create_folder`
- `rename_folder`
- `move_folder`
- `copy_folder`
- `delete_folder`

### Board, tag, and workspace-structure write tools

- `create_board`
- `update_board`
- `delete_board`
- `create_column`
- `update_column`
- `delete_column`
- `create_tag`
- `update_tag`
- `delete_tag`
- `create_project`
- `update_project`
- `delete_project`
- `create_status_template`
- `update_status_template`
- `delete_status_template`
- `create_saved_search`
- `rename_saved_search`
- `delete_saved_search`
- `create_task_view`
- `update_task_view`
- `delete_task_view`

## Recommended agent workflow

1. use `search` or the list tools to discover targets first
2. use task readable IDs and document slugs in follow-up calls
3. for document edits, call `get_document` with `detail=full`, keep the returned revision id, then call `update_document_content`
4. if a CAS conflict comes back, apply the returned patch and retry against `current_revision_id`
5. only call destructive tools after an explicit decision and `confirm: true`

## Current MCP gaps

Compared with REST, MCP intentionally omits several areas:

- no prompts capability
- no user/admin management tools
- no API-key management tools
- no group, grant, or property-definition tools
- no workspace create/update/admin-delete tools
- no webhook, integration-config, or automation-rule tools
- no attachment upload/download/delete tools; the current attachment surface is document-attachment metadata listing only
