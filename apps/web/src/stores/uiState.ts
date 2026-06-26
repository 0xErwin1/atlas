import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types';
import { wrappedClient } from '@/api/wrapper';
import type { TaskBoardView } from '@/stores/ui';

// The server models `state` as an opaque JSON object, so the generated type is
// an empty object. We hold concrete keys (e.g. collapsedFolders), so the PUT
// body is cast to the wire type at the boundary.
type UiStatePayload = components['schemas']['UpdateUiStateRequest']['state'];

/**
 * Per-user UI state, persisted server-side via `/v1/me/ui-state` so preferences
 * (e.g. which sidebar folders are collapsed) survive refreshes and follow the
 * user across devices. Writes are debounced into a single PUT.
 */
export const useUiStateStore = defineStore('uiState', () => {
  const data = ref<Record<string, unknown>>({});
  const loaded = ref(false);

  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  async function load(): Promise<void> {
    const { data: res } = await wrappedClient.GET('/v1/me/ui-state');
    const state = (res as { state?: unknown } | undefined)?.state;
    if (state !== null && typeof state === 'object') {
      data.value = state as Record<string, unknown>;
    }
    loaded.value = true;
  }

  function scheduleSave(): void {
    if (saveTimer !== null) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      saveTimer = null;
      void wrappedClient.PUT('/v1/me/ui-state', {
        body: { state: data.value as unknown as UiStatePayload },
      });
    }, 600);
  }

  // Expanded sidebar folders are stored as a list of ids; absence means the
  // folder is collapsed (the default), so a fresh user sees the tree closed and
  // opens only what they need.
  function expandedFolders(): string[] {
    const v = data.value.expandedFolders;
    return Array.isArray(v) ? (v as string[]) : [];
  }

  function isFolderCollapsed(id: string): boolean {
    return !expandedFolders().includes(id);
  }

  function setFolderCollapsed(id: string, collapsed: boolean): void {
    const next = new Set(expandedFolders());
    if (collapsed) next.delete(id);
    else next.add(id);
    data.value = { ...data.value, expandedFolders: [...next] };
    scheduleSave();
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

  return {
    data,
    loaded,
    load,
    isFolderCollapsed,
    setFolderCollapsed,
    boardViewFor,
    setBoardView,
  };
});
