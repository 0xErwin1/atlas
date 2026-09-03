import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';
import { isBoardView } from '@/lib/boardViews';
import type { TaskBoardView } from '@/stores/ui';

// The server models `state` as an opaque JSON object, so the generated type is
// an empty object. We hold concrete keys (e.g. collapsedFolders), so the PUT
// body is cast to the wire type at the boundary.
type UiStatePayload = components['schemas']['UpdateUiStateRequest']['state'];

interface SidebarExpansionState {
  collapsedProjects: string[];
  expandedFolders: string[];
}

/**
 * Per-user UI state, persisted server-side via `/api/me/ui-state` so preferences
 * (e.g. which sidebar folders are collapsed) survive refreshes and follow the
 * user across devices. Writes are debounced and serialized so an older PUT can
 * never finish after a newer PUT from this client.
 */
export const useUiStateStore = defineStore('uiState', () => {
  const data = ref<Record<string, unknown>>({});
  const loaded = ref(false);
  const error = ref<string | null>(null);

  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let saveLoop: Promise<void> | null = null;
  let saveRequested = false;
  let latestSnapshot: Record<string, unknown> = {};
  let generation = 0;

  async function load(): Promise<void> {
    const requestGeneration = generation;
    const { data: res, error: loadError } = await wrappedClient.GET('/api/v2/platform/me/ui-state');
    if (requestGeneration !== generation) return;

    if (loadError !== undefined) {
      error.value = errorHint(loadError, 'Failed to load UI preferences');
      loaded.value = true;
      return;
    }

    const state = (res as { state?: unknown } | undefined)?.state;
    if (state !== null && typeof state === 'object') {
      data.value = state as Record<string, unknown>;
    }
    error.value = null;
    loaded.value = true;
  }

  function scheduleSave(): void {
    latestSnapshot = data.value;
    if (saveTimer !== null) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      saveTimer = null;
      saveRequested = true;
      ensureSaveLoop();
    }, 600);
  }

  function ensureSaveLoop(): void {
    if (saveLoop !== null) return;

    const loopGeneration = generation;
    saveLoop = runSaveLoop(loopGeneration);
    void saveLoop.finally(() => {
      saveLoop = null;
      if (saveRequested) ensureSaveLoop();
    });
  }

  async function runSaveLoop(loopGeneration: number): Promise<void> {
    while (loopGeneration === generation && saveRequested) {
      saveRequested = false;
      await save(latestSnapshot, loopGeneration);
    }
  }

  async function save(state: Record<string, unknown>, requestGeneration: number): Promise<void> {
    try {
      const { error: saveError } = await wrappedClient.PUT('/api/v2/platform/me/ui-state', {
        body: { state: state as unknown as UiStatePayload },
      });
      if (requestGeneration !== generation) return;

      error.value = saveError === undefined ? null : errorHint(saveError, 'Failed to save UI preferences');
    } catch (cause) {
      if (requestGeneration === generation) {
        error.value = errorHint(cause, 'Failed to save UI preferences');
      }
    }
  }

  function legacyExpandedFolders(): string[] {
    const v = data.value.expandedFolders;
    return Array.isArray(v) ? (v as string[]) : [];
  }

  function sidebarExpansionByWorkspace(): Record<string, SidebarExpansionState> {
    const value = data.value.sidebarExpansionByWorkspace;
    if (value === null || typeof value !== 'object' || Array.isArray(value)) return {};

    return value as Record<string, SidebarExpansionState>;
  }

  function workspaceExpansion(workspaceId: string): SidebarExpansionState {
    const stored = sidebarExpansionByWorkspace()[workspaceId];
    return {
      collapsedProjects: Array.isArray(stored?.collapsedProjects) ? stored.collapsedProjects : [],
      expandedFolders: Array.isArray(stored?.expandedFolders)
        ? stored.expandedFolders
        : legacyExpandedFolders(),
    };
  }

  function setWorkspaceExpansion(workspaceId: string, state: SidebarExpansionState): void {
    data.value = {
      ...data.value,
      sidebarExpansionByWorkspace: {
        ...sidebarExpansionByWorkspace(),
        [workspaceId]: state,
      },
    };
    scheduleSave();
  }

  function isProjectCollapsed(workspaceId: string, projectId: string): boolean {
    return workspaceExpansion(workspaceId).collapsedProjects.includes(projectId);
  }

  function setProjectCollapsed(workspaceId: string, projectId: string, collapsed: boolean): void {
    const state = workspaceExpansion(workspaceId);
    const next = new Set(state.collapsedProjects);
    if (collapsed) next.add(projectId);
    else next.delete(projectId);

    setWorkspaceExpansion(workspaceId, { ...state, collapsedProjects: [...next] });
  }

  function isFolderCollapsed(workspaceId: string, folderId: string): boolean {
    return !workspaceExpansion(workspaceId).expandedFolders.includes(folderId);
  }

  function setFolderCollapsed(workspaceId: string, folderId: string, collapsed: boolean): void {
    const state = workspaceExpansion(workspaceId);
    const next = new Set(state.expandedFolders);
    if (collapsed) next.delete(folderId);
    else next.add(folderId);

    setWorkspaceExpansion(workspaceId, { ...state, expandedFolders: [...next] });
  }

  function reset(): void {
    generation += 1;
    saveRequested = false;
    latestSnapshot = {};
    if (saveTimer !== null) {
      clearTimeout(saveTimer);
      saveTimer = null;
    }

    data.value = {};
    loaded.value = false;
    error.value = null;
  }

  // The board layout (kanban/list/table/...) the user last chose, keyed by board
  // id. Absence means the board has no saved preference and falls back to the
  // default view.
  function boardViews(): Record<string, TaskBoardView> {
    const v = data.value.boardViews;
    return v !== null && typeof v === 'object' ? (v as Record<string, TaskBoardView>) : {};
  }

  function boardViewFor(boardId: string): TaskBoardView | undefined {
    return boardViews()[boardId];
  }

  function setBoardView(boardId: string, view: TaskBoardView): void {
    data.value = {
      ...data.value,
      boardViews: { ...boardViews(), [boardId]: view },
    };
    scheduleSave();
  }

  // Which list-view groups the user has collapsed, keyed by board id. Server
  // backed like the sidebar's collapse state: a collapsed group is a statement
  // about how the user reads that board, and losing it on every navigation is
  // what made the collapse feel like it did not stick.
  function collapsedListGroups(): Record<string, string[]> {
    const v = data.value.collapsedListGroups;
    return v !== null && typeof v === 'object' ? (v as Record<string, string[]>) : {};
  }

  function isListGroupCollapsed(boardId: string, groupKey: string): boolean {
    return (collapsedListGroups()[boardId] ?? []).includes(groupKey);
  }

  function setListGroupCollapsed(boardId: string, groupKey: string, collapsed: boolean): void {
    const groups = collapsedListGroups();
    const next = new Set(groups[boardId] ?? []);
    if (collapsed) next.add(groupKey);
    else next.delete(groupKey);

    data.value = {
      ...data.value,
      collapsedListGroups: { ...groups, [boardId]: [...next] },
    };
    scheduleSave();
  }

  // A layout pinned from settings, applied to every board the user opens. Absence
  // means no preference, so each board falls back to its own remembered layout.
  function defaultBoardView(): TaskBoardView | null {
    const v = data.value.defaultBoardView;
    return isBoardView(v) ? v : null;
  }

  function setDefaultBoardView(view: TaskBoardView | null): void {
    const next = { ...data.value };

    if (view === null) delete next.defaultBoardView;
    else next.defaultBoardView = view;

    data.value = next;
    scheduleSave();
  }

  return {
    data,
    loaded,
    error,
    load,
    reset,
    isProjectCollapsed,
    setProjectCollapsed,
    isFolderCollapsed,
    setFolderCollapsed,
    isListGroupCollapsed,
    setListGroupCollapsed,
    boardViewFor,
    setBoardView,
    defaultBoardView,
    setDefaultBoardView,
  };
});
