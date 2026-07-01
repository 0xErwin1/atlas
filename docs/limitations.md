# Limitations and details

This page records caveats backed by the current code, tests, or authoritative repo docs.

## Documentation and contributor caveats

- `ARCHITECTURE.md` is authoritative for system structure, but exact route coverage is best checked in `crates/atlas_server/src/routes/registry.rs` and `/openapi.json`.
- `ROUTE_REGISTRY` only enforces registry → router and registry → OpenAPI drift checks. A route added to the Axum router without a registry entry is not auto-detected.
- Exact schema details should be checked in `crates/migration`, not inferred from prose summaries.

## Auth, permissions, and tenancy

- bearer auth takes precedence over the session cookie
- cookie-authenticated mutations require `X-Atlas-CSRF: 1`; bearer-authenticated mutations do not
- API keys are capped at editor authority and do not receive the root/system-admin bypass that human users can have
- some permission failures intentionally return `404` instead of `403` to avoid disclosing resource existence
- the workspace “last owner” invariant is enforced, but the check is not fully atomic; two simultaneous demotions/removals of the last two owners could theoretically race

## REST API specifics

- search cursors are sort-aware; reusing a cursor with a different sort returns an input error
- ordinary page cursors and search cursors are different opaque formats
- search with task-only filters against note-only scope can legitimately return an empty page
- document attachment upload uses raw bytes plus headers, while task attachment upload uses multipart
- the integration ingest endpoint is intentionally public but HMAC-verified and capped at `1 MiB`
- several REST routes are not wrapped by `atlas_client`, including activation, system-admin toggle, webhooks, integration configs, and automation rules

## Resource lifecycle and soft-delete behavior

- workspace deletion is soft-delete: rows are hidden from lookups but preserved
- project deletion soft-deletes the project row; boards, tasks, and documents inside are not cascaded and become unreachable from normal listings
- board deletion soft-deletes only the board row; columns and tasks remain in storage but disappear from normal board access paths
- folder deletion can leave documents with their existing `folder_id`, effectively orphaned from normal navigation
- deleting tags, status templates, groups, property definitions, automation rules, integration configs, and webhooks is implemented as soft-delete of the owning row
- deleting a task or document attachment removes the DB row; content-addressed blob storage may remain because the same bytes can still be referenced elsewhere

## Documents and notes

- document content writes are compare-and-swap, so clients must handle `409` revision conflicts
- the web app canonicalizes UUID-addressed note routes back to slug URLs after load
- notes autosave is timer-based; overlapping edits can require explicit CAS conflict resolution in the UI
- recursive folder copy is intentionally depth-limited (`32`) to avoid pathological or cyclic trees

## Tasks and boards

- subtasks are full tasks, not lightweight child rows; they are excluded from board listings until promoted
- checklist items also still exist as a separate lighter-weight concept and can be promoted into tasks
- column deletion is refused when the column still contains tasks
- task assignees must still be valid for the workspace; disabled users are rejected, while pending (not yet activated) users remain assignable
- task attachment uploads stop streaming as soon as they exceed the configured size cap; the default cap is `20 MiB`

## Search, tags, and custom structure

- tag creation is idempotent by case-insensitive name
- deleting a tag does not scrub existing task label strings already stored on tasks
- tag color cannot currently be cleared through the MCP tool surface; supply a new color or leave it unchanged
- property-definition options are only valid for `select` and `multi_select`; other kinds must omit them entirely
- saved searches and task views are per-owner resources with duplicate-name conflicts and per-owner caps enforced server-side

## Web app specifics

- the Share dialog's general-access section is still read-only
- the “Ask AI” dialog builds a copyable prompt only; Atlas does not execute an LLM request from that dialog
- the web app depends on generated OpenAPI types; after backend contract changes, `just gen-types` is required

## CLI specifics

- there is no `atlas login` command
- destructive commands often require explicit confirmation flags such as `--confirm` or `--yes`
- board resolution can use a case-insensitive substring match, which is convenient but can be ambiguous
- JSON output is automatic whenever stdout is not a TTY
- the CLI does not expose every REST route; notable omissions include activation, webhooks, integration configs, automation rules, and the root-only system-admin toggle

## MCP specifics

- MCP advertises tools and resources, not prompts
- MCP intentionally exposes an agent-focused subset and omits user/admin management, API-key management, grants, groups, property definitions, webhook/integration/automation admin flows, and workspace create/update/admin-delete
- current MCP attachment coverage is metadata-only for document and task attachments; it does not expose attachment upload/download/delete tools

## Automation and webhook specifics

- integration config secrets are returned exactly once
- webhook signing secrets are returned exactly once
- webhook secrets cannot currently be rotated through a dedicated REST endpoint
- integration config creation in v1 only supports the `github` integration slug
- automation rules are intentionally narrow in v1: trigger event types must start with `external.`, and the only action type is `create_task`
- webhook URL validation checks for a non-empty absolute `http`/`https` URL with a host, but does not prove reachability at creation time
