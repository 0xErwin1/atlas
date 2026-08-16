import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import type { AiAction, AiPromptTask } from '@/lib/aiPrompt';

export type InspectorTab = 'properties' | 'backlinks' | 'comments' | 'activity' | 'share';
export type BannerType = 'error' | 'warning' | 'info' | 'success';
export type Theme = 'dark' | 'light';
export type TaskViewMode = 'sidebar' | 'modal' | 'full';
export type EditorMode = 'live' | 'source';
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

// A toast auto-dismisses after this long. Errors linger longer than confirmations
// because they carry information the user may need to read and act on.
const BANNER_TIMEOUT_MS: Record<BannerType, number> = {
  success: 4000,
  info: 4000,
  warning: 6000,
  error: 8000,
};

const INSPECTOR_STORAGE_KEY = 'atlas:inspector';
export const EDITOR_WIDE_STORAGE_KEY = 'atlas:editor-wide';
export const EDITOR_READING_STORAGE_KEY = 'atlas:editor-reading';
export const EDITOR_MODE_STORAGE_KEY = 'atlas:editor-mode';
export const EDITOR_LINE_NUMBERS_STORAGE_KEY = 'atlas:editor-line-numbers';
export const THEME_STORAGE_KEY = 'atlas:theme';
const SIDEBAR_STORAGE_KEY = 'atlas:sidebar-collapsed';
export const TASK_VIEW_MODE_STORAGE_KEY = 'atlas.taskview.mode';
const TASK_INSPECTOR_STORAGE_KEY = 'atlas:task-inspector-open';

function loadTaskInspectorOpen(): boolean {
  try {
    return localStorage.getItem(TASK_INSPECTOR_STORAGE_KEY) !== '0';
  } catch {
    return true;
  }
}

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

function loadEditorReading(): boolean {
  try {
    return localStorage.getItem(EDITOR_READING_STORAGE_KEY) === '1';
  } catch {
    return false;
  }
}

// Line numbers default on: a note is a line-addressed document elsewhere in
// Atlas — `read_document_lines` and `edit_document_lines` speak in line numbers
// — and without the gutter there is no way to see the numbers those refer to.
function loadEditorLineNumbers(): boolean {
  try {
    return localStorage.getItem(EDITOR_LINE_NUMBERS_STORAGE_KEY) !== '0';
  } catch {
    return true;
  }
}

function loadEditorMode(): EditorMode {
  try {
    if (localStorage.getItem(EDITOR_MODE_STORAGE_KEY) === 'source') return 'source';
  } catch {
    // ignore malformed storage
  }
  return 'live';
}

function persistSetting(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // ignore storage errors
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

  // Editor view mode, shared by every note the user opens. Held here rather than
  // in the notes view because that view is unmounted whenever a board takes over
  // the router outlet, which would otherwise drop the choice on the way back.
  const editorReading = ref(loadEditorReading());
  const editorMode = ref<EditorMode>(loadEditorMode());
  const editorLineNumbers = ref(loadEditorLineNumbers());

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

  // Applies a theme change made in another tab (from a `storage` event). Updates
  // the reactive state and the DOM but never re-persists, so it cannot bounce a
  // fresh `storage` event back to the tab that made the original change.
  function applyExternalTheme(raw: string): void {
    if (raw === 'light' || raw === 'dark') {
      theme.value = raw;
      applyTheme(raw);
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

  let bannerTimer: ReturnType<typeof setTimeout> | null = null;

  function clearBannerTimer() {
    if (bannerTimer !== null) {
      clearTimeout(bannerTimer);
      bannerTimer = null;
    }
  }

  function showBanner(message: string, type: BannerType) {
    clearBannerTimer();
    banner.value = { message, type };
    bannerTimer = setTimeout(() => {
      banner.value = null;
      bannerTimer = null;
    }, BANNER_TIMEOUT_MS[type]);
  }

  function dismissBanner() {
    clearBannerTimer();
    banner.value = null;
  }

  function toggleEditorWide() {
    editorWide.value = !editorWide.value;
    persistSetting(EDITOR_WIDE_STORAGE_KEY, editorWide.value ? '1' : '0');
  }

  // Mirrors an editor-width change from another tab without re-persisting. Any
  // value other than '1' is read as false, matching `loadEditorWide`.
  function applyExternalEditorWide(raw: string): void {
    editorWide.value = raw === '1';
  }

  // Setters rather than toggles alone: the editor writes both through v-model,
  // and a write that bypassed these would leave the choice unpersisted.
  function setEditorReading(value: boolean) {
    editorReading.value = value;
    persistSetting(EDITOR_READING_STORAGE_KEY, value ? '1' : '0');
  }

  function toggleEditorReading() {
    setEditorReading(!editorReading.value);
  }

  function applyExternalEditorReading(raw: string): void {
    editorReading.value = raw === '1';
  }

  function setEditorMode(value: EditorMode) {
    editorMode.value = value;
    persistSetting(EDITOR_MODE_STORAGE_KEY, value);
  }

  function toggleEditorMode() {
    setEditorMode(editorMode.value === 'source' ? 'live' : 'source');
  }

  // Anything other than 'source' is read as 'live', matching `loadEditorMode`.
  function applyExternalEditorMode(raw: string): void {
    editorMode.value = raw === 'source' ? 'source' : 'live';
  }

  function setEditorLineNumbers(value: boolean) {
    editorLineNumbers.value = value;
    persistSetting(EDITOR_LINE_NUMBERS_STORAGE_KEY, value ? '1' : '0');
  }

  function toggleEditorLineNumbers() {
    setEditorLineNumbers(!editorLineNumbers.value);
  }

  // Anything other than '0' is read as on, matching `loadEditorLineNumbers`.
  function applyExternalEditorLineNumbers(raw: string): void {
    editorLineNumbers.value = raw !== '0';
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
  const shortcutsHelpOpen = ref(false);

  function openPalette() {
    paletteOpen.value = true;
  }

  function closePalette() {
    paletteOpen.value = false;
  }

  function togglePalette() {
    paletteOpen.value = !paletteOpen.value;
  }

  function openShortcutsHelp() {
    shortcutsHelpOpen.value = true;
  }

  function closeShortcutsHelp() {
    shortcutsHelpOpen.value = false;
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

  // A one-off presentation override, e.g. the context menu's "Open as…": it
  // applies to the task being opened now without touching the persisted default.
  // Session-only — never written to localStorage, and dropped on a normal open.
  const taskViewModeOverride = ref<TaskViewMode | null>(null);

  const effectiveTaskViewMode = computed<TaskViewMode>(
    () => taskViewModeOverride.value ?? taskViewMode.value,
  );

  function setTaskViewMode(mode: TaskViewMode) {
    taskViewMode.value = mode;
    taskViewModeOverride.value = null;
    try {
      localStorage.setItem(TASK_VIEW_MODE_STORAGE_KEY, mode);
    } catch {
      // ignore storage errors
    }
  }

  // Mirrors a persisted task-view-mode change from another tab without
  // re-persisting. Leaves this tab's session-only override untouched; malformed
  // values are ignored, matching `loadTaskViewMode`.
  function applyExternalTaskViewMode(raw: string): void {
    if (raw === 'sidebar' || raw === 'modal' || raw === 'full') {
      taskViewMode.value = raw;
    }
  }

  function openTaskInMode(mode: TaskViewMode) {
    taskViewModeOverride.value = mode;
  }

  function clearTaskViewModeOverride() {
    taskViewModeOverride.value = null;
  }

  // Whether the full-screen task detail's right inspector dock is shown. Persisted
  // so collapsing it for more body width sticks across tasks/sessions.
  const taskInspectorOpen = ref(loadTaskInspectorOpen());

  function toggleTaskInspector() {
    taskInspectorOpen.value = !taskInspectorOpen.value;
    try {
      localStorage.setItem(TASK_INSPECTOR_STORAGE_KEY, taskInspectorOpen.value ? '1' : '0');
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

  // Free-text board finder, kept separate from the structured facets above so the
  // quick-search input and the filter panel never overwrite each other. Session
  // only; matched against task title and readable id by `filteredTasksByColumn`.
  const taskFilterText = ref('');

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

  function setTaskFilterText(text: string): void {
    taskFilterText.value = text;
  }

  function clearTaskFilter(): void {
    taskFilter.value = { statuses: [], priorities: [], assigneeIds: [], labels: [] };
    taskFilterText.value = '';
  }

  // "Ask AI" hand-off dialog. Global so it can be opened both from the task
  // detail banner and from a task row's context menu, while a single instance is
  // mounted in the app shell. The task snapshot carries whatever fields the
  // caller has (the summary lacks the description); the prompt builder copes.
  const askAiOpen = ref(false);
  const askAiTask = ref<AiPromptTask | null>(null);
  const askAiStatus = ref<string | null>(null);
  const askAiAction = ref<AiAction>('summarize');

  function openAskAi(task: AiPromptTask, statusName: string | null, action: AiAction) {
    askAiTask.value = task;
    askAiStatus.value = statusName;
    askAiAction.value = action;
    askAiOpen.value = true;
  }

  function closeAskAi() {
    askAiOpen.value = false;
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
    applyExternalEditorWide,
    editorReading,
    setEditorReading,
    toggleEditorReading,
    applyExternalEditorReading,
    editorMode,
    setEditorMode,
    toggleEditorMode,
    applyExternalEditorMode,
    editorLineNumbers,
    setEditorLineNumbers,
    toggleEditorLineNumbers,
    applyExternalEditorLineNumbers,
    theme,
    setTheme,
    applyExternalTheme,
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
    shortcutsHelpOpen,
    openShortcutsHelp,
    closeShortcutsHelp,
    sidebarCollapsed,
    toggleSidebar,
    taskViewMode,
    effectiveTaskViewMode,
    setTaskViewMode,
    applyExternalTaskViewMode,
    openTaskInMode,
    clearTaskViewModeOverride,
    taskInspectorOpen,
    toggleTaskInspector,
    taskView,
    setTaskView,
    taskGroupBy,
    setTaskGroupBy,
    taskFilter,
    taskFilterText,
    hasActiveFilter,
    setTaskFilter,
    setTaskFilterText,
    clearTaskFilter,
    askAiOpen,
    askAiTask,
    askAiStatus,
    askAiAction,
    openAskAi,
    closeAskAi,
  };
});
