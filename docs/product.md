# Product and features

Atlas combines markdown knowledge management, kanban task management, and agent-friendly automation inside multi-workspace boundaries.

## What Atlas is

Atlas has four main surfaces built on the same backend:

| Surface | Audience | Notes |
|---|---|---|
| Web app | Humans | Vue SPA for notes, tasks, search, and settings. |
| REST API | Integrators | The product contract; every client uses it. |
| CLI | Scripts and terminal users | Thin wrapper over `atlas_client`. |
| MCP server | Agents and MCP hosts | Agent-focused subset over the same REST API. |

## Core resource model

| Resource | Purpose | Notes |
|---|---|---|
| Workspace | Tenant boundary and collaboration root | Creating a workspace seeds a default project, status templates, and a default board. |
| Project | Groups notes, folders, boards, and project-scoped sharing | Has a stable slug and a task ID prefix. |
| Folder | Organizes documents inside a project | Can be moved and recursively copied. |
| Document | Markdown note | Tracks revisions, frontmatter, backlinks, and attachments. |
| Board | Kanban board inside a project | Holds ordered columns and top-level tasks. |
| Column | Board status lane | Used as the current status of a task. |
| Task | Work item with readable ID like `ATL-42` | Supports assignees, labels, properties, references, attachments, checklist, subtasks, and activity. |
| Grant | Resource-sharing permission entry | Targets users, API keys, and groups. |
| Group | Workspace principal group | Reusable principal for permission grants. |
| API key | Script/agent credential | User-owned; agent reach is capped at editor. |
| Webhook | Outbound signed event subscription | Admin-only workspace feature. |
| Integration config | External event-ingest config | Currently provisions GitHub-compatible signed ingest. |
| Automation rule | External-event rule | v1 supports external triggers that create tasks. |

## Main product capabilities

### Identity, access, and administration

- Username/password login for human users.
- Invite-and-activate flow for pending users.
- Session cookies for the browser and bearer tokens for scripts/agents.
- Profile update and password change.
- Root-only user creation, disable/enable, password reset, activation-link regeneration, and system-admin promotion.
- Workspace memberships with `owner`, `admin`, and `member` roles.
- Resource sharing through grants on workspaces and projects.
- User-owned API keys, including optional initial workspace grants and global reach toggles.
- Workspace groups for reusable grant targets.
- Workspace and platform security audit feeds.

### Notes and knowledge base

- Markdown is the source of truth for documents.
- Document endpoints accept a slug or a stable UUID on read/update paths that target a document.
- Revisions are compare-and-swap: writes carry a base revision and return conflicts instead of silently overwriting.
- Revision history and individual revision content are exposed.
- Frontmatter is parsed and exposed as structured JSON.
- Wikilinks/backlinks connect documents and tasks.
- Documents can be moved, copied, attached to folders, and uploaded with binary attachments.
- Folder trees support creation, rename, move, recursive copy, and delete.

### Tasks and planning

- Projects hold boards, columns, and tasks.
- Tasks have immutable readable IDs derived from the project prefix at creation time.
- Task fields include title, description, priority, estimate, due date, labels, and custom properties.
- Tasks support assignees (users and API keys), outbound references, inbound backlinks, attachments, and per-task activity.
- Checklist items can be added, reordered, updated, removed, and promoted to tasks.
- Subtasks are full tasks linked to a parent and can be promoted to top-level tasks.
- Workspace-wide task listing supports filters and saved task views.
- Workspace status templates can be applied to a board.

### Search and saved views

- Workspace search covers both notes and tasks.
- Search supports free text plus filter tokens and explicit `type`, `sort`, `cursor`, `limit`, and `prefix` query parameters.
- Saved searches are per-owner workspace objects.
- Task views are per-owner named filter presets.
- Tags and property definitions provide additional structure around tasks and documents.

### Automation, integrations, and webhooks

- Integration configs create one-time secrets and provision an integration API key.
- Public event ingest is HMAC-verified.
- Automation rules are workspace or project scoped.
- Webhooks deliver signed outbound events and keep delivery logs.

## Web app capabilities

The web app lives in `apps/web/src` and currently exposes these top-level routes:

| Route | Main purpose |
|---|---|
| `/login` | Sign in |
| `/activate/:token` | Activation flow for invited users |
| `/n/:slug?` | Notes workspace |
| `/t/:boardId?` | Board-centric task workspace |
| `/t/views/:viewId` | Saved/predefined task views |
| `/t/task/:readableId` | Full-screen task detail |
| `/search` | Cross-workspace search UI |
| `/settings/:section?` | Account, workspace, and admin panels |

Notable frontend features backed by `apps/web/src/views` and `apps/web/src/components`:

- Notes sidebar, tab strip, backlinks/history inspector panels, wikilink autocomplete, and autosaving markdown editor.
- CAS conflict resolution UI for concurrent document edits.
- Task views in board, list, table, calendar, and timeline layouts.
- Task detail pane and full-screen task detail route.
- Command palette (`Cmd/Ctrl+K`) and search preview.
- Settings panels for account, API keys, workspace general settings, statuses, default status templates, tags, projects, members, groups, workspace activity, workspace audit, users, admin workspace management, platform audit, and about.
- “Ask AI” dialog that builds a copyable prompt from a task; it does not run a model in Atlas itself.

## Interface coverage at a glance

| Capability area | Web | REST | CLI | MCP |
|---|---:|---:|---:|---:|
| Auth/session/profile | Yes | Yes | Partial | No |
| Notes/documents | Yes | Yes | Yes | Yes |
| Boards/tasks | Yes | Yes | Yes | Yes |
| Search | Yes | Yes | Yes | Yes |
| Workspace/project sharing | Yes | Yes | Yes | No |
| User/admin management | Yes | Yes | Yes | No |
| API-key management | Yes | Yes | Yes | No |
| Webhooks/integrations/automation | REST-first | Yes | No | No |

See [limitations.md](limitations.md) for gaps and intentionally partial areas.
