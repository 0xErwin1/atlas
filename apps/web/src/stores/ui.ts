import { defineStore } from 'pinia';
import { computed, ref } from 'vue';

export type InspectorTab = 'properties' | 'backlinks' | 'activity' | 'share';
export type BannerType = 'error' | 'warning' | 'info' | 'success';
export type Theme = 'dark' | 'light';
export type TaskViewMode = 'sidebar' | 'modal' | 'full';
export type TaskBoardView = 'board' | 'list' | 'table' | 'calendar' | 'timeline';
export type TaskGroupBy = 'status' | 'assignee' | 'priority';

export interface TaskFilterState {
  statuses: string[];
  priorities: string[];
  assigneeIds: string[];
  labels: string[];
}

export interface Banner {
  message: string;
  type: BannerType;
}

const INSPECTOR_STORAGE_KEY = 'atlas:inspector';
const EDITOR_WIDE_STORAGE_KEY = 'atlas:editor-wide';
const THEME_STORAGE_KEY = 'atlas:theme';
const SIDEBAR_STORAGE_KEY = 'atlas:sidebar-collapsed';
const TASK_VIEW_MODE_STORAGE_KEY = 'atlas.taskview.mode';

function loadTaskViewMode(): TaskViewMode {
  try {
    const v = localStorage.getItem(TASK_VIEW_MODE_STORAGE_KEY);
    if (v === 'sidebar' || v === 'modal' || v === 'full') return v;
  } catch {
    // ignore malformed storage
  }
  return 'sidebar';
}

function loadSidebarCollapsed(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_STORAGE_KEY) === '1';
  } catch {
    return false;
  }
}

function loadTheme(): Theme {
  try {
    const v = localStorage.getItem(THEME_STORAGE_KEY);
    if (v === 'light' || v === 'dark') return v;
  } catch {
    // ignore malformed storage
  }
  return 'dark';
}

function applyTheme(theme: Theme): void {
  try {
    document.documentElement.dataset.theme = theme;
  } catch {
    // no document (non-browser context)
  }
}

function loadInspectorState(): { open: boolean; tab: InspectorTab } {
  try {
    const raw = localStorage.getItem(INSPECTOR_STORAGE_KEY);
    if (raw) return JSON.parse(raw) as { open: boolean; tab: InspectorTab };
  } catch {
    // ignore malformed storage
  }
  return { open: false, tab: 'properties' };
}

function loadEditorWide(): boolean {
  try {
    return localStorage.getItem(EDITOR_WIDE_STORAGE_KEY) === '1';
  } catch {
    return false;
  }
}

export const useUiStore = defineStore('ui', () => {
  const saved = loadInspectorState();

  const inspectorOpen = ref(saved.open);
  const inspectorTab = ref<InspectorTab>(saved.tab);
  const banner = ref<Banner | null>(null);

  const shareOpen = ref(false);
  const shareResourceLabel = ref('');
  const shareProjectSlug = ref<string | null>(null);

  // Editor reading width: false = readable column, true = full viewport width.
  const editorWide = ref(loadEditorWide());

  const theme = ref<Theme>(loadTheme());
  applyTheme(theme.value);

  function setTheme(next: Theme) {
    theme.value = next;
    applyTheme(next);
    try {
      localStorage.setItem(THEME_STORAGE_KEY, next);
    } catch {
      // ignore storage errors
    }
  }

  function persistInspector() {
    try {
      localStorage.setItem(
        INSPECTOR_STORAGE_KEY,
        JSON.stringify({ open: inspectorOpen.value, tab: inspectorTab.value }),
      );
    } catch {
      // ignore storage errors
    }
  }

  function toggleInspector() {
    inspectorOpen.value = !inspectorOpen.value;
    persistInspector();
  }

  function setInspectorTab(tab: InspectorTab) {
    inspectorTab.value = tab;
    persistInspector();
  }

  function showBanner(message: string, type: BannerType) {
    banner.value = { message, type };
  }

  function dismissBanner() {
    banner.value = null;
  }

  function toggleEditorWide() {
    editorWide.value = !editorWide.value;
    try {
      localStorage.setItem(EDITOR_WIDE_STORAGE_KEY, editorWide.value ? '1' : '0');
    } catch {
      // ignore storage errors
    }
  }

  function openShare(resourceLabel: string, projectSlug?: string) {
    shareResourceLabel.value = resourceLabel;
    shareProjectSlug.value = projectSlug ?? null;
    shareOpen.value = true;
  }

  function closeShare() {
    shareOpen.value = false;
    shareProjectSlug.value = null;
  }

  const paletteOpen = ref(false);

  function openPalette() {
    paletteOpen.value = true;
  }

  function closePalette() {
    paletteOpen.value = false;
  }

  function togglePalette() {
    paletteOpen.value = !paletteOpen.value;
  }

  const sidebarCollapsed = ref(loadSidebarCollapsed());

  function toggleSidebar() {
    sidebarCollapsed.value = !sidebarCollapsed.value;
    try {
      localStorage.setItem(SIDEBAR_STORAGE_KEY, sidebarCollapsed.value ? '1' : '0');
    } catch {
      // ignore storage errors
    }
  }

  // How an opened task is presented: a right-side dock, a floating dialog, or
  // full screen. Persisted so the user's preference sticks across tasks/sessions.
  const taskViewMode = ref<TaskViewMode>(loadTaskViewMode());

  function setTaskViewMode(mode: TaskViewMode) {
    taskViewMode.value = mode;
    try {
      localStorage.setItem(TASK_VIEW_MODE_STORAGE_KEY, mode);
    } catch {
      // ignore storage errors
    }
  }

  // Which layout the board's tasks render in (kanban board, list, table,
  // calendar, timeline) and how non-board layouts group rows. Session state.
  const taskView = ref<TaskBoardView>('board');

  function setTaskView(view: TaskBoardView) {
    taskView.value = view;
  }

  const taskGroupBy = ref<TaskGroupBy>('status');

  function setTaskGroupBy(group: TaskGroupBy) {
    taskGroupBy.value = group;
  }

  // Session-only filter state for the tasks board. Parallels taskGroupBy and
  // taskView: ephemeral, not persisted, cleared when the board unmounts or the
  // user explicitly clears it.
  const taskFilter = ref<TaskFilterState>({
    statuses: [],
    priorities: [],
    assigneeIds: [],
    labels: [],
  });

  const hasActiveFilter = computed<boolean>(
    () =>
      taskFilter.value.statuses.length > 0 ||
      taskFilter.value.priorities.length > 0 ||
      taskFilter.value.assigneeIds.length > 0 ||
      taskFilter.value.labels.length > 0,
  );

  function setTaskFilter(next: TaskFilterState): void {
    taskFilter.value = next;
  }

  function clearTaskFilter(): void {
    taskFilter.value = { statuses: [], priorities: [], assigneeIds: [], labels: [] };
  }

  return {
    inspectorOpen,
    inspectorTab,
    banner,
    shareOpen,
    shareResourceLabel,
    shareProjectSlug,
    editorWide,
    toggleEditorWide,
    theme,
    setTheme,
    toggleInspector,
    setInspectorTab,
    showBanner,
    dismissBanner,
    openShare,
    closeShare,
    paletteOpen,
    openPalette,
    closePalette,
    togglePalette,
    sidebarCollapsed,
    toggleSidebar,
    taskViewMode,
    setTaskViewMode,
    taskView,
    setTaskView,
    taskGroupBy,
    setTaskGroupBy,
    taskFilter,
    hasActiveFilter,
    setTaskFilter,
    clearTaskFilter,
  };
});
