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

HTTP mode mounts the MCP service at `/mcp`. It is stateless: responses do not issue an
`Mcp-Session-Id`, and stale session headers are ignored so requests continue across restarts and
load-balanced instances.

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
- `update` distinguishes omitted fields from explicit `null`
- `delete` requires `confirm: true`
- document content edits are CAS-based and return structured conflict data
- write resources resolve boards/columns by name and return actionable errors listing valid options on misses

## Tool catalog

Tools are verbs, not one tool per capability. Each takes a `resource` naming what it acts on
and a `params` object carrying that resource's own arguments:

```json
{ "resource": "tasks", "params": { "workspace": "atlas", "status": "open" } }
```

The advertised tools are `find`, `get`, `create`, `update`, `delete`, `move`, `document_edit`,
`comment`, `attachment`, `activity`, `identity`, and `help`.

| Verb | Covers |
| --- | --- |
| `find` | every listing and search: `search`, `tasks`, `documents`, `folders`, `boards`, `columns`, `tags`, `used_labels`, `members`, `workspaces`, `projects`, `saved_searches`, `task_views`, `checklist`, `status_templates`, `platform_status_templates`, `webhooks`, `webhook_deliveries` |
| `get` | one resource by identifier: `document`, `task`, `task_references`, `task_graph`, `task_backlinks`, `document_backlinks`, `document_revision`, `webhook` |
| `create` | `task`, `subtask`, `document`, `document_copy`, `folder`, `folder_copy`, `board`, `column`, `tag`, `project`, `status_template`, `platform_status_template`, `saved_search`, `task_view`, `webhook`, `task_assignee`, `task_reference`, `task_references_batch`, `checklist_item` |
| `update` | `task`, `document_metadata`, `board`, `column`, `tag`, `checklist_item`, `project`, `status_template`, `platform_status_template`, `task_view`, `webhook`, `folder_name`, `saved_search_name` |
| `delete` | the same resources as `create`, plus `task_assignee` and `task_reference` |
| `move` | `task`, `document`, `documents_batch`, `folder`, `task_parent`, `checklist_item_promotion`, `subtask_promotion` |
| `document_edit` | the content path: `read_lines`, `search_content`, `edit_lines`, `replace_content` |
| `comment` | `task_list`, `task_feed`, `task_add`, `task_update`, `task_delete`, and the `document_*` equivalents |
| `attachment` | `document_list`, `task_list`, `task_get`, and the comment-attachment upload/list/get/delete pairs for tasks and documents |
| `activity` | `task`, `workspace`, `workspace_audit`, `platform_audit`, `document_history` |
| `identity` | `ping`, `agent` |

### Discovering what a resource takes

`help` is the lookup, so the per-resource schemas stay out of every client's context until they
are needed:

- `help` with no arguments lists every verb and its resources
- `help` with a verb summarizes that verb's resources and their parameters
- `help` with a verb and a resource returns the resource's full JSON Schema
- `help` with a pre-consolidation tool name as the verb answers with where that capability moved

An invalid call carries the same information: an unknown resource is answered with the accepted
resource names, and a bad `params` object with the accepted parameter set. A wrong guess costs
one round trip, not a failed session.

Consolidating took the advertised `tools/list` payload from 112 tools and ~93 KB to 12 tools
and ~13 KB, measured on the same server. A test bounds it, so the saving cannot be spent by
moving the per-resource detail back into the tool descriptions.

The catalog itself lives in `crates/atlas_mcp/src/catalog.rs`; every entry records the name the
capability was advertised under before the consolidation, and a test asserts none of them
became unreachable.

## Recommended agent workflow

1. use `find` with `resource: "search"` for discovery; it reads the query and picks lexical retrieval for filter tokens or a quoted phrase and hybrid (lexical + embeddings) for prose, and `mode` overrides that when you know better
2. use task readable IDs and document slugs in follow-up calls
3. before implementing a task, call `get` `task` with `detail=full` and `attachment` `task_list` so descriptions and attached screenshots/files are considered
4. to locate a passage inside a large document, call `document_edit` `search_content` for matching line ranges, then `read_lines` to pull only those lines; both return `head_revision_id`
5. for a targeted edit, call `document_edit` `edit_lines` with `base_revision_id` set to that `head_revision_id` — it replaces a bounded line range without resending the whole body
6. for a whole-document rewrite, call `get` `document` with `detail=full`, keep the returned `head_revision_id`, then call `document_edit` `replace_content` with `base_revision_id`
7. if a CAS conflict comes back from either write path, apply the returned patch and retry against `current_revision_id`
8. when reordering columns, checklist items, or status templates, read the current `position_key` values from the matching `find` resource: `before` is the position key of the item the moved item will follow, `after` is the position key of the item it will precede
9. only call `delete` after an explicit decision and `confirm: true`

## Current MCP gaps

Compared with REST, MCP intentionally omits several areas:

- no prompts capability
- no user/admin management resources
- no API-key management resources
- no group, grant, or property-definition tools
- no workspace create/update/admin-delete tools
- no webhook, integration-config, or automation-rule tools
- no attachment upload/download/delete tools; the current attachment surface is metadata listing for document attachments and task attachments
