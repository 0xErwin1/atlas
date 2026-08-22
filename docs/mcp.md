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
load-balanced instances. Only `POST` is served; `GET` and `DELETE` answer `405`.

## Protocol revisions

Atlas implements two eras and serves both from the same endpoint:

| Revision | Era | Notes |
|---|---|---|
| `2026-07-28` | modern | per-request lifecycle, `server/discover`, SEP-2243 routing headers, SEP-2549 cache hints, SEP-2322 `resultType` and MRTR |
| `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05` | legacy | `initialize` handshake, no per-request metadata, no `resultType` |

The list lives in one place: `SUPPORTED_PROTOCOL_VERSIONS`, returned by
`ServerHandler::supported_protocol_versions` in `crates/atlas_mcp/src/lib.rs`. It is pinned there
rather than inherited from rmcp's `KNOWN_VERSIONS`, so an SDK upgrade cannot advertise a revision
Atlas has not been built and tested against.

### Dual-era compatibility policy

Which era a request gets is decided by rmcp's own version gating against that list, never by
hand-rolled branching in a tool body. A request selects its era the way the spec says it does:

- `initialize` negotiates from `params.protocolVersion`; the answer is the client's version when
  Atlas implements it, otherwise Atlas's own.
- A modern request instead carries `MCP-Protocol-Version` and repeats the version in
  `_meta.io.modelcontextprotocol/protocolVersion`. No `initialize` is needed.
- A request with neither is read as `2025-03-26`, which is the legacy default the spec prescribes.

Consequences of the split, all enforced by the SDK:

- **Required request metadata.** A `2026-07-28` request must carry both
  `io.modelcontextprotocol/protocolVersion` and `io.modelcontextprotocol/clientCapabilities` in
  `_meta`. A missing or malformed key is `-32602` with HTTP 400. Legacy requests require neither.
- **Routing headers (SEP-2243).** On `2026-07-28`, `Mcp-Method` must match the body method, and
  `Mcp-Name` must match `params.name` for `tools/call` and `params.uri` for `resources/read`. A
  value that cannot travel as a bare header arrives wrapped as `=?base64?<base64>?=` and is decoded
  before comparison. A mismatch, a missing required header, or a `MCP-Protocol-Version` that
  disagrees with `_meta` is `-32020` with HTTP 400. Legacy requests are not checked.
- **Unknown revision.** A version Atlas does not implement is `-32022` with HTTP 400.
- **Unknown method** is `-32601` with HTTP 404 on the modern path.
- **Missing resource.** `resources/read` on a document the caller cannot see is `-32002` for legacy
  peers and `-32602` for `2026-07-28` peers (SEP-2164).
- **`resultType`.** Modern results carry `"complete"`; legacy results carry no discriminator.

`server/discover` answers without an `initialize`, listing the supported revisions and the same
capabilities `initialize` reports.

### Caching (SEP-2549)

Atlas advertises no freshness window: every list and read is served from a live Atlas API call whose
result can change on the next request, so `ttlMs` is always `0` and no response may be replayed
without revalidating. The fields appear only on `2026-07-28` responses, because older revisions have
nowhere to put them.

| Result | `cacheScope` | Why |
|---|---|---|
| `server/discover` | `private` | rmcp's default for the discovery result |
| `tools/list` | `public` | the catalog is identical for every caller |
| `resources/templates/list` | `public` | one static template, identical for every caller |
| `resources/read` | `private` | a document body is scoped to the calling principal |

Tool ordering in `tools/list` is deterministic: the router emits the catalog sorted by tool name, so
the same server build always answers with the same sequence and the response is safe to cache.

### Confirming a destructive delete (SEP-2322)

`delete` refuses to run without `confirm: true`. On `2026-07-28`, a caller that declared the
`elicitation` client capability gets that refusal as a multi round-trip request instead of an error
string: the result is `resultType: "input_required"` carrying one `elicitation/create` request that
names the target, the client asks the human, and the retry repeats the same `tools/call` with the
answer in `inputResponses`. An accepted answer runs the delete; anything else cancels it and deletes
nothing.

Atlas emits no `requestState` and trusts none. The retry resends its own arguments, so an echoed
value could never authorize more than the caller can already ask for directly, and there is nothing
to integrity-protect.

Every other caller — a legacy peer, or a modern peer that cannot answer an elicitation — keeps the
original actionable error asking it to re-call with `confirm: true`. Atlas never advertises
elicitation itself; it only uses what the caller says it supports.

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

Because HTTP mode issues no session and resolves the bearer token per request, two consecutive
requests carrying different tokens act as two different principals. Nothing about the first
survives into the second.

### Transport-level checks

`/mcp` in HTTP mode also enforces, before any handler runs:

- **`Host`** must name the bind address or a loopback name (`127.0.0.1`, `::1`, `localhost`),
  otherwise `403`. This is the DNS-rebinding guard for a locally reachable server.
- **`Origin`**, when present, must name one of the server's own `http://<host>:<port>` origins,
  otherwise `403`. Agents and the CLI send no `Origin`, so this only rejects browsers.

Behind the bundled nginx, the `/mcp` location sets `proxy_set_header Host localhost`, so every
proxied request satisfies the host check regardless of the public hostname. The check therefore
protects direct access to the MCP port, not access through the proxy.

## Advertised capabilities

`atlas_mcp` advertises:

- tools
- resources

It advertises nothing else. In particular there are no prompts, no completions, no logging, no
resource subscriptions, and no `io.modelcontextprotocol/tasks` extension — Atlas product tasks are
ordinary MCP tool results, not MCP protocol tasks. Sampling, elicitation and roots are client
capabilities; Atlas implements none of them as a server and only sends the confirmation elicitation
described above to a client that declared it can answer one.

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

Protocol features Atlas deliberately does not implement, on either era:

- no prompts, completions or logging capability
- no resource subscriptions, in either the legacy `resources/subscribe` form or the `2026-07-28`
  `subscriptions/listen` form
- no `io.modelcontextprotocol/tasks` extension; a long Atlas operation is a plain tool call, and an
  Atlas product task is domain data, not an MCP protocol task
- no server-initiated sampling or roots requests
- no elicitation beyond the single destructive-delete confirmation, and only to a client that
  declared it can answer one
- no `Mcp-Param-*` header promotion; no tool input schema carries an `x-mcp-header` annotation
