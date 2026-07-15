import { defineStore } from 'pinia';
import { computed, readonly, ref } from 'vue';
import type { components } from '@/api/types';
import { type CacheInvalidationScope, wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';
import { collectPaged } from '@/lib/pagination';
import { disposeWorkspaceLiveUpdates } from '@/lib/workspaceLiveUpdates';
import { useAuthStore } from '@/stores/auth';
import { useLastViewedStore } from '@/stores/lastViewed';

type ProjectDto = components['schemas']['Page_ProjectDto']['items'][number];

export type WorkspaceDto = components['schemas']['WorkspaceDto'];
export type PrincipalDto = components['schemas']['PrincipalDto'];
export type UserDto = components['schemas']['UserDto'];

export interface ProjectSummary {
  slug: string;
  name: string;
  task_prefix: string;
  workspace_id: string;
  visibility: string;
}

const WORKSPACE_STORAGE_KEY = 'atlas:workspace';
const PENDING_INVALIDATION_MAX_AGE_MS = 5 * 60_000;
const PENDING_INVALIDATION_MAX_ENTRIES = 100;

type WorkspaceAliasInvalidationHandler = (
  scope: CacheInvalidationScope,
  workspaceId: string,
) => Promise<boolean>;

let workspaceAliasInvalidationHandler: WorkspaceAliasInvalidationHandler | undefined;

export function setWorkspaceAliasInvalidationHandler(handler: WorkspaceAliasInvalidationHandler): void {
  workspaceAliasInvalidationHandler = handler;
}

function loadStoredWorkspace(): string | null {
  try {
    return localStorage.getItem(WORKSPACE_STORAGE_KEY);
  } catch {
    return null;
  }
}

function persistWorkspace(slug: string): void {
  try {
    localStorage.setItem(WORKSPACE_STORAGE_KEY, slug);
  } catch {
    // ignore storage errors
  }
}

export const useWorkspaceStore = defineStore('workspace', () => {
  const activeWorkspaceSlug = ref<string | null>(null);
  // The last non-null active workspace. Unlike `activeWorkspaceSlug`, it is not
  // cleared while a switch briefly nulls the active slug, so a failed switch can
  // revert to the workspace that was actually committed last rather than to the
  // transient null an overlapping switch may have left behind.
  const committedSlug = ref<string | null>(null);
  // Concurrency-safe switch tracking: the active slug is briefly null during a
  // switch, and `switching` lets the router guards tell that transient null apart
  // from a cold start so they neither bootstrap from localStorage nor record the
  // restored resource under the wrong workspace. Each switch takes a monotonic
  // token from `beginSwitch` and a matching `endSwitch` in its `finally`;
  // `switching` stays true until every in-flight switch has settled (an in-flight
  // count, so an earlier switch's completion never clears the flag while a later
  // overlapping switch is still navigating). `isCurrentSwitch` lets a switch tell
  // whether it is still the latest before it reverts or commits its target, so a
  // superseded switch never clobbers a newer one's outcome.
  let switchGeneration = 0;
  const activeSwitchCount = ref(0);
  const switching = computed(() => activeSwitchCount.value > 0);
  const projects = ref<ProjectSummary[]>([]);
  const workspaces = ref<WorkspaceDto[]>([]);
  // Every workspace in the system, loaded on demand for the root-only admin
  // panel. Kept separate from `workspaces` (the caller's own memberships).
  const adminWorkspaces = ref<WorkspaceDto[]>([]);
  const members = ref<PrincipalDto[]>([]);
  // Users eligible to be added to the active workspace (not already members and
  // not disabled). Populated on demand by the add-member dialog.
  const assignableUsers = ref<UserDto[]>([]);
  const error = ref<string | null>(null);
  const workspaceAliases = new Map<string, string>();
  const pendingCacheInvalidations = new Map<
    string,
    {
      scope: CacheInvalidationScope;
      resolvers: Array<(invalidated: boolean) => void>;
      timeout: ReturnType<typeof setTimeout>;
    }
  >();
  let membersLoadRequest: object | null = null;
  let workspaceLoadGeneration = 0;

  const auth = useAuthStore();

  /**
   * The signed-in user's role in the active workspace, derived from the loaded
   * member list (the server does not put the workspace role on the auth user).
   * Null when the user has no membership row here — which is the case for a
   * break-glass global admin who never joined this workspace.
   */
  const myWorkspaceRole = computed<'owner' | 'admin' | 'member' | null>(() => {
    const id = auth.user?.id;
    if (id == null) return null;

    const self = members.value.find((m) => m.id === id);
    const role = self?.role;
    if (role === 'owner' || role === 'admin' || role === 'member') return role;
    return null;
  });

  function setActiveWorkspace(slug: string | null) {
    if (slug !== activeWorkspaceSlug.value) {
      membersLoadRequest = null;
      members.value = [];
    }
    activeWorkspaceSlug.value = slug;
    if (slug !== null) committedSlug.value = slug;
  }

  function isCurrentWorkspaceRequest(requestGeneration: number, requestSessionGeneration: number): boolean {
    return (
      requestGeneration === workspaceLoadGeneration && requestSessionGeneration === auth.sessionGeneration
    );
  }

  function stagedWorkspaceAliases(items: readonly WorkspaceDto[], replace: boolean): Map<string, string> {
    const aliases = replace ? new Map<string, string>() : new Map(workspaceAliases);
    for (const workspace of items) aliases.set(workspace.slug, workspace.id);
    return aliases;
  }

  function publishWorkspaceAliases(aliases: ReadonlyMap<string, string>): void {
    workspaceAliases.clear();
    for (const [slug, id] of aliases) workspaceAliases.set(slug, id);
  }

  function workspaceIdForSlug(slug: string): string | null {
    return workspaceAliases.get(slug) ?? null;
  }

  function clearWorkspaceAliases(): void {
    workspaceLoadGeneration += 1;
    workspaceAliases.clear();
    settleAllPendingCacheInvalidations(false);
  }

  function queueCacheInvalidation(scope: CacheInvalidationScope): Promise<boolean> {
    if (scope.workspaceSlug === null || scope.scope === 'none') return Promise.resolve(true);

    const key = `${scope.workspaceSlug}|${scope.scope}|${scope.tags.join(',')}`;
    const pending = pendingCacheInvalidations.get(key);

    return new Promise((resolve) => {
      if (pending !== undefined) {
        pending.resolvers.push(resolve);
        return;
      }

      const timeout = setTimeout(
        () => settlePendingCacheInvalidation(key, false),
        PENDING_INVALIDATION_MAX_AGE_MS,
      );
      pendingCacheInvalidations.set(key, { scope, resolvers: [resolve], timeout });

      while (pendingCacheInvalidations.size > PENDING_INVALIDATION_MAX_ENTRIES) {
        const oldest = pendingCacheInvalidations.keys().next().value;
        if (oldest === undefined) return;
        settlePendingCacheInvalidation(oldest, false);
      }
    });
  }

  async function flushPendingCacheInvalidations(aliases: ReadonlyMap<string, string>): Promise<void> {
    const handler = workspaceAliasInvalidationHandler;
    if (!handler) return;

    for (const [key, pending] of pendingCacheInvalidations) {
      const workspaceId = pending.scope.workspaceSlug ? aliases.get(pending.scope.workspaceSlug) : undefined;
      if (workspaceId === undefined) continue;

      try {
        settlePendingCacheInvalidation(key, await handler(pending.scope, workspaceId));
      } catch {
        settlePendingCacheInvalidation(key, false);
      }
    }
  }

  function settlePendingCacheInvalidation(key: string, invalidated: boolean): void {
    const pending = pendingCacheInvalidations.get(key);
    if (pending === undefined) return;

    clearTimeout(pending.timeout);
    pendingCacheInvalidations.delete(key);
    for (const resolve of pending.resolvers) resolve(invalidated);
  }

  function settleAllPendingCacheInvalidations(invalidated: boolean): void {
    for (const key of pendingCacheInvalidations.keys()) settlePendingCacheInvalidation(key, invalidated);
  }

  /** Starts a switch and returns its token, to be passed back to `endSwitch`. */
  function beginSwitch(): number {
    activeSwitchCount.value += 1;
    switchGeneration += 1;
    return switchGeneration;
  }

  /**
   * Ends a switch, decrementing the in-flight count so `switching` clears once all
   * settle. The sole invariant is the 1-begin/1-end `finally` pairing; the `token`
   * is accepted only for symmetry with `beginSwitch`/`isCurrentSwitch` and is unused.
   */
  function endSwitch(_token: number): void {
    if (activeSwitchCount.value > 0) {
      activeSwitchCount.value -= 1;
    }
  }

  /** True only when `token` is still the latest switch — no newer switch has started. */
  function isCurrentSwitch(token: number): boolean {
    return token === switchGeneration;
  }

  async function loadWorkspaces(): Promise<string | null> {
    const requestGeneration = ++workspaceLoadGeneration;
    const requestSessionGeneration = auth.sessionGeneration;
    const { data, error } = await wrappedClient.GET('/api/workspaces');

    if (!isCurrentWorkspaceRequest(requestGeneration, requestSessionGeneration)) return null;

    if (error !== undefined || data === undefined) {
      const hint = (error as { hint?: string } | undefined)?.hint;
      console.error('loadWorkspaces failed', hint ?? error);
      return null;
    }

    const aliases = stagedWorkspaceAliases(data, true);
    await flushPendingCacheInvalidations(aliases);
    if (!isCurrentWorkspaceRequest(requestGeneration, requestSessionGeneration)) return null;

    publishWorkspaceAliases(aliases);
    workspaces.value = data;

    // Restore the last-used workspace when it still exists, otherwise the first.
    const stored = loadStoredWorkspace();
    const chosen = data.find((w) => w.slug === stored) ?? data[0];
    if (chosen !== undefined) {
      setActiveWorkspace(chosen.slug);
      return chosen.slug;
    }

    return null;
  }

  /**
   * Switches the active workspace: clears the cached project list so consumers
   * (watching `activeWorkspaceSlug`) reload for the new workspace, and persists
   * the choice so it survives a refresh.
   */
  function switchWorkspace(slug: string): void {
    if (slug === committedSlug.value) {
      if (activeWorkspaceSlug.value !== slug) setActiveWorkspace(slug);
      return;
    }
    if (committedSlug.value !== null) disposeWorkspaceLiveUpdates();
    setActiveWorkspace(slug);
    projects.value = [];
    persistWorkspace(slug);
  }

  /**
   * Creates a workspace and switches to it. Returns the new slug, or null on
   * failure (with `error` set).
   */
  async function createWorkspace(name: string): Promise<string | null> {
    const requestGeneration = ++workspaceLoadGeneration;
    const requestSessionGeneration = auth.sessionGeneration;
    const { data, error: apiError } = await wrappedClient.POST('/api/workspaces', {
      body: { name },
    });

    if (!isCurrentWorkspaceRequest(requestGeneration, requestSessionGeneration)) return null;

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create workspace');
      return null;
    }

    const list = await wrappedClient.GET('/api/workspaces');
    if (!isCurrentWorkspaceRequest(requestGeneration, requestSessionGeneration)) return null;

    if (list.data !== undefined) {
      const aliases = stagedWorkspaceAliases(list.data, true);
      await flushPendingCacheInvalidations(aliases);
      if (!isCurrentWorkspaceRequest(requestGeneration, requestSessionGeneration)) return null;

      publishWorkspaceAliases(aliases);
      workspaces.value = list.data;
    }

    switchWorkspace(data.slug);
    return data.slug;
  }

  async function loadProjects(ws: string): Promise<void> {
    const { items, error } = await collectPaged<ProjectDto>((cursor) =>
      wrappedClient.GET('/api/workspaces/{ws}/projects', {
        params: { path: { ws }, query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) } },
      }),
    );

    if (error !== undefined) {
      projects.value = [];
      return;
    }

    projects.value = items.map((p) => ({
      slug: p.slug,
      name: p.name,
      task_prefix: p.task_prefix,
      workspace_id: p.workspace_id,
      visibility: p.visibility,
    }));
  }

  /**
   * Creates a project from a display name, deriving a URL slug and an uppercase
   * task prefix (both required by the API). Returns the new project's slug, or
   * null on failure (with `error` set).
   */
  async function createProject(ws: string, name: string): Promise<string | null> {
    const slug =
      name
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/(^-|-$)/g, '')
        .slice(0, 40) || 'project';
    const taskPrefix =
      name
        .toUpperCase()
        .replace(/[^A-Z0-9]/g, '')
        .slice(0, 4) || 'PRJ';

    const { data, error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/projects', {
      params: { path: { ws } },
      body: { name, slug, task_prefix: taskPrefix, visibility: 'workspace' },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create project');
      return null;
    }

    await loadProjects(ws);
    return data.slug ?? slug;
  }

  /** Renames a project. Returns true on success; sets `error` and returns false otherwise. */
  async function renameProject(ws: string, slug: string, name: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/projects/{project_slug}', {
      params: { path: { ws, project_slug: slug } },
      body: { name },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to rename project');
      return false;
    }

    await loadProjects(ws);
    return true;
  }

  /**
   * Updates a project's name, task prefix, and/or visibility. Pass only the fields that should
   * change; the server ignores absent fields. Returns true on success and sets
   * `error` (surfacing the API `hint`) on failure — callers show banners for 409
   * duplicate-prefix and 422 bad-format responses.
   */
  async function updateProject(
    ws: string,
    slug: string,
    patch: { name?: string; task_prefix?: string; visibility?: 'private' | 'workspace' | 'public' },
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/projects/{project_slug}', {
      params: { path: { ws, project_slug: slug } },
      body: patch,
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to update project');
      return false;
    }

    await loadProjects(ws);
    return true;
  }

  /**
   * Deletes a project and everything under it (boards, folders, documents).
   * Returns true on success; sets `error` and returns false otherwise.
   */
  async function deleteProject(ws: string, slug: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/api/workspaces/{ws}/projects/{project_slug}', {
      params: { path: { ws, project_slug: slug } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete project');
      return false;
    }

    await loadProjects(ws);
    return true;
  }

  /**
   * Renames a workspace's display name (the slug is never re-derived server-side,
   * so existing links stay valid). Reflects the new name in any cached workspace
   * list — the active memberships and, when present, the admin list. Returns true
   * on success; sets `error` and returns false otherwise.
   */
  async function renameWorkspace(ws: string, name: string): Promise<boolean> {
    const requestGeneration = ++workspaceLoadGeneration;
    const requestSessionGeneration = auth.sessionGeneration;
    const { data, error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}', {
      params: { path: { ws } },
      body: { name },
    });

    if (!isCurrentWorkspaceRequest(requestGeneration, requestSessionGeneration)) return false;

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to rename workspace');
      return false;
    }

    const apply = (list: WorkspaceDto[]): WorkspaceDto[] => list.map((w) => (w.slug === ws ? data : w));

    const aliases = stagedWorkspaceAliases([data], false);
    await flushPendingCacheInvalidations(aliases);
    if (!isCurrentWorkspaceRequest(requestGeneration, requestSessionGeneration)) return false;

    publishWorkspaceAliases(aliases);
    workspaces.value = apply(workspaces.value);
    adminWorkspaces.value = apply(adminWorkspaces.value);
    return true;
  }

  /**
   * Re-slugs a workspace via the admin endpoint (root/system-admin only). Reflects
   * the new slug in any cached list, and if the active workspace was re-slugged,
   * repoints the active slug and the persisted choice so navigation keeps working.
   * Returns true on success; sets `error` and returns false otherwise (e.g. 422
   * for an invalid or already-taken slug).
   */
  async function updateWorkspaceSlug(ws: string, slug: string): Promise<boolean> {
    const requestGeneration = ++workspaceLoadGeneration;
    const requestSessionGeneration = auth.sessionGeneration;
    const { data, error: apiError } = await wrappedClient.PATCH('/api/admin/workspaces/{ws}', {
      params: { path: { ws } },
      body: { slug },
    });

    if (!isCurrentWorkspaceRequest(requestGeneration, requestSessionGeneration)) return false;

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update workspace slug');
      return false;
    }

    const apply = (list: WorkspaceDto[]): WorkspaceDto[] => list.map((w) => (w.slug === ws ? data : w));

    const aliases = stagedWorkspaceAliases([data], false);
    aliases.delete(ws);
    await flushPendingCacheInvalidations(aliases);
    if (!isCurrentWorkspaceRequest(requestGeneration, requestSessionGeneration)) return false;

    publishWorkspaceAliases(aliases);
    workspaces.value = apply(workspaces.value);
    adminWorkspaces.value = apply(adminWorkspaces.value);

    useLastViewedStore().rekey(ws, data.slug);

    if (activeWorkspaceSlug.value === ws) {
      disposeWorkspaceLiveUpdates();
      activeWorkspaceSlug.value = data.slug;
      committedSlug.value = data.slug;
      persistWorkspace(data.slug);
    }

    return true;
  }

  /**
   * Soft-deletes a workspace via the admin endpoint (root/system-admin only).
   * Drops it from the cached lists; if it was the active workspace, switches to
   * the first remaining one (or clears the selection). Returns true on success;
   * sets `error` and returns false otherwise.
   */
  async function deleteWorkspace(ws: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/api/admin/workspaces/{ws}', {
      params: { path: { ws } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete workspace');
      return false;
    }

    workspaces.value = workspaces.value.filter((w) => w.slug !== ws);
    adminWorkspaces.value = adminWorkspaces.value.filter((w) => w.slug !== ws);
    workspaceAliases.delete(ws);

    // Drop the deleted workspace's stored resource so it is never restored.
    useLastViewedStore().clear(ws);

    if (activeWorkspaceSlug.value === ws) {
      const next = workspaces.value[0]?.slug ?? null;
      disposeWorkspaceLiveUpdates();
      setActiveWorkspace(next);
      projects.value = [];
      if (next !== null) persistWorkspace(next);
    }

    return true;
  }

  /**
   * Loads every workspace in the system for the root-only admin panel. Clears the
   * list and sets `error` on failure (e.g. a non-root caller gets 403).
   */
  async function loadAdminWorkspaces(): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET('/api/admin/workspaces');

    if (apiError !== undefined || data === undefined) {
      adminWorkspaces.value = [];
      error.value = errorHint(apiError, 'Failed to load workspaces');
      return;
    }

    adminWorkspaces.value = data;
  }

  /** Loads workspace members (users and agents) for assignee pickers. */
  async function loadMembers(ws: string): Promise<void> {
    const request = {};
    membersLoadRequest = request;

    try {
      const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/members', {
        params: { path: { ws } },
      });

      if (membersLoadRequest !== request || activeWorkspaceSlug.value !== ws) return;

      membersLoadRequest = null;
      if (apiError !== undefined || data === undefined) {
        members.value = [];
        error.value = errorHint(apiError, 'Failed to load members');
        return;
      }

      members.value = data;
    } catch (cause) {
      if (membersLoadRequest !== request || activeWorkspaceSlug.value !== ws) return;

      membersLoadRequest = null;
      members.value = [];
      error.value = errorHint(cause, 'Failed to load members');
    }
  }

  /**
   * Changes a workspace member's role (owner/admin/member) and reloads the
   * member list on success. Surfaces the backend `hint` in `error` for the
   * panel to display — the server is authoritative, so a 403 (insufficient
   * privileges) or 409 (`urn:atlas:error:last-owner`) is reported as-is.
   * Returns true on success.
   */
  async function updateMemberRole(ws: string, userId: string, role: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/members/{user_id}', {
      params: { path: { ws, user_id: userId } },
      body: { role },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to update member role');
      return false;
    }

    await loadMembers(ws);
    return true;
  }

  /**
   * Removes a member from the workspace and reloads the member list on success.
   * Surfaces the backend `hint` in `error` (e.g. 403 not allowed, 409 last
   * owner). Returns true on success.
   */
  async function removeMember(ws: string, userId: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/api/workspaces/{ws}/members/{user_id}', {
      params: { path: { ws, user_id: userId } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to remove member');
      return false;
    }

    await loadMembers(ws);
    return true;
  }

  /**
   * Loads the users that can be added to the workspace (not already members and
   * not disabled) into `assignableUsers`. Clears the list on failure so the
   * add-member picker never shows stale candidates.
   */
  async function loadAssignableUsers(ws: string): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/assignable-users', {
      params: { path: { ws } },
    });

    if (apiError !== undefined || data === undefined) {
      assignableUsers.value = [];
      return;
    }

    assignableUsers.value = data;
  }

  /**
   * Adds an existing user to the workspace at the given role. Surfaces the
   * backend `hint` in `error` on failure (e.g. 403 admin granting owner, 404
   * user not found, 409 already a member, 422 disabled user). Returns true on
   * success — the caller reloads the member list.
   */
  async function addMember(ws: string, userId: string, role: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/members', {
      params: { path: { ws } },
      body: { user_id: userId, role },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to add member');
      return false;
    }

    return true;
  }

  return {
    activeWorkspaceSlug,
    committedSlug: readonly(committedSlug),
    switching,
    projects,
    workspaces,
    adminWorkspaces,
    members,
    assignableUsers,
    myWorkspaceRole,
    error,
    setActiveWorkspace,
    clearWorkspaceAliases,
    queueCacheInvalidation,
    workspaceIdForSlug,
    beginSwitch,
    endSwitch,
    isCurrentSwitch,
    switchWorkspace,
    createWorkspace,
    renameWorkspace,
    updateWorkspaceSlug,
    deleteWorkspace,
    loadWorkspaces,
    loadAdminWorkspaces,
    loadProjects,
    createProject,
    renameProject,
    updateProject,
    deleteProject,
    loadMembers,
    updateMemberRole,
    removeMember,
    loadAssignableUsers,
    addMember,
  };
});
