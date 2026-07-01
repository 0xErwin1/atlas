# Web app

The Atlas web app is a Vue 3 SPA in `apps/web`. It is one first-class consumer of the REST API and uses generated OpenAPI types through `openapi-fetch`.

## Routes

| Route | Purpose |
|-------|---------|
| `/login` | Login screen. Authenticated users redirect away. |
| `/activate/:token` | Invitation activation and password setup. |
| `/n/:slug?` | Notes/documents workspace. |
| `/t/:boardId?` | Tasks/boards workspace. |
| `/t/views/:viewId` | Saved task view. |
| `/t/task/:readableId` | Full task detail route. |
| `/search` | Search page. |
| `/settings/:section?` | Settings pages. |

The router guard skips auth for activation, redirects authenticated users away from login, fetches `/v1/auth/me`, loads workspaces, and then loads persisted UI state.

## Shell and navigation

- `AppShell` owns the responsive layout: rail, sidebar, main area, inspector, share dialog, and AI prompt dialog.
- `AppRail` provides Notes, Tasks, Search, settings, workspace switch/create, and logout.
- Cmd/Ctrl+K opens the command palette.

## Notes

The notes UI supports:

- project and folder navigation,
- document tabs,
- markdown editing through CodeMirror,
- live/source/readonly preview modes,
- 800ms autosave,
- compare-and-swap conflict handling,
- backlinks,
- history/revisions,
- frontmatter split/join,
- folder/document create, rename, move, copy, and delete.

Wikilinks:

- `[[uuid|Title]]` is the stable id-bound form.
- `[[Title]]` remains the legacy title-only form.
- Autocomplete searches notes with `type=note&limit=8`.
- If autocomplete fails, the UI degrades to a create/free-typed title path.
- UUID-addressed note URLs canonicalize to slug URLs after load.

## Tasks

The task UI supports:

- board, list, table, calendar, and timeline layouts,
- filters and grouping by status/assignee/priority where applicable,
- inline task detail on larger layouts,
- standalone full task route on mobile/full mode,
- title/status/assignee/due/priority/estimate/tag editing,
- descriptions,
- custom fields,
- subtasks,
- checklist items,
- attachments,
- references/dependencies,
- activity,
- copy-prompt AI actions.

Date-oriented layouts may lazy-load full task DTOs because task summaries do not include every date field needed by those views.

## Search and command palette

- `/search` exposes workspace search, filters, and saved searches.
- The command palette combines local actions with search-driven navigation.
- Search input is debounced through shared composables.

## Sharing and settings

Sharing:

- Global share dialog manages workspace grants by default.
- It manages project grants when called with a project slug.
- Candidates merge workspace members, caller API keys, and groups.
- API keys/agents are capped at viewer/editor in the UI.
- General access is display-only; changing visibility there is not available yet.

Settings include:

- account,
- API keys,
- workspace general settings,
- statuses/default statuses,
- tags,
- projects,
- members,
- groups,
- activity/audit,
- root/system-admin user and workspace administration,
- platform audit,
- about.

Visibility gates hide root/system-admin and workspace-admin sections from users without the required role.

## Frontend contributor patterns

| Concern | Pattern |
|---------|---------|
| API | Use `wrappedClient` over generated OpenAPI types. |
| Credentials | `wrappedClient` sends cookies and CSRF headers for unsafe methods. |
| State | Use Pinia stores by domain. |
| Forms | Use zod and shared validation helpers. |
| API errors | Surface the API `hint` through shared error helpers. |
| Shared UI | Reuse primitives such as Dropdown, Popover, ConfirmDialog, FormField, EmptyState, and settings table components. |
| Generated types | Run `just gen-types`; do not hand-edit generated `types.d.ts`. |

## Product notes

- The AI dialog copies a prepared prompt; it does not call a model.
- General-access visibility toggling is not implemented in the share dialog.
- Notes autosave is conflict-safe but overlapping conflicts need user resolution.
- Task summaries are intentionally lighter than full task DTOs.
