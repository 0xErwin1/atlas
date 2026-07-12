<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import EditorToolbar from '@/components/shell/EditorToolbar.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import BoardViewMenu from '@/components/tareas/BoardViewMenu.vue';
import KanbanBoard from '@/components/tareas/KanbanBoard.vue';
import TaskCalendarView from '@/components/tareas/TaskCalendarView.vue';
import TaskDetailPane from '@/components/tareas/TaskDetailPane.vue';
import TaskFilterPanel from '@/components/tareas/TaskFilterPanel.vue';
import TaskListView from '@/components/tareas/TaskListView.vue';
import TaskTableView from '@/components/tareas/TaskTableView.vue';
import TaskTimelineView from '@/components/tareas/TaskTimelineView.vue';
import TaskViewListView from '@/components/tareas/TaskViewListView.vue';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import PresenceAvatars from '@/components/ui/PresenceAvatars.vue';
import { useBoardPresence } from '@/composables/useBoardPresence';
import { useBreakpoint } from '@/composables/useBreakpoint';
import { installKeymapListener, useKeymap } from '@/composables/useKeymap';
import { type LiveUpdateEvent, useLiveUpdates } from '@/composables/useLiveUpdates';
import { useOpenTaskLive } from '@/composables/useOpenTaskLive';
import { EVENT_TYPE, eventString, PRESENCE_UPDATED } from '@/lib/eventTypes';
import { KEYMAP_PRIORITIES } from '@/lib/keymap';
import { useBoardsStore } from '@/stores/boards';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useTasksStore } from '@/stores/tasks';
import { useTaskViewsStore } from '@/stores/taskViews';
import type { TaskGroupBy, TaskViewMode } from '@/stores/ui';
import { useUiStore } from '@/stores/ui';
import { useUiStateStore } from '@/stores/uiState';
import { useWorkspaceStore } from '@/stores/workspace';
import { paramsForView, useWorkspaceTasksStore } from '@/stores/workspaceTasks';
import AppShell from '@/views/AppShell.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import TasksSidebar from '@/views/TasksSidebar.vue';

const route = useRoute();
const router = useRouter();
const workspace = useWorkspaceStore();
const boards = useBoardsStore();
const tasks = useTasksStore();
const detail = useTaskDetailStore();
const ui = useUiStore();
const uiState = useUiStateStore();
const workspaceTasks = useWorkspaceTasksStore();
const taskViews = useTaskViewsStore();
const lastViewed = useLastViewedStore();
const { isMobile } = useBreakpoint();

// A restored board that no longer exists loads as a 404: show an empty state,
// not an error, and stop restoring the dead entry on the next workspace switch.
const boardNotFound = computed(() => boards.loadError !== null && boards.loadErrorStatus === 404);

const boardId = computed(() => {
  const id = route.params.boardId;
  return typeof id === 'string' ? id : null;
});

const viewId = computed(() => {
  const id = route.params.viewId;
  return typeof id === 'string' ? id : null;
});

const isView = computed(() => viewId.value !== null);

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

// Quick board finder: a "/" from anywhere on the board (while not already typing)
// focuses the search input, mirroring the search-shortcut convention. Esc in the
// input clears the query, then blurs on a second press.
const boardSearchRef = ref<HTMLInputElement | null>(null);

function onSearchKeydownEsc(event: KeyboardEvent): void {
  event.preventDefault();
  event.stopPropagation();

  if (ui.taskFilterText !== '') {
    ui.setTaskFilterText('');
  } else if (event.target instanceof HTMLInputElement) {
    event.target.blur();
  }
}

function clearBoardSearch(): void {
  ui.setTaskFilterText('');
  boardSearchRef.value?.focus();
}

const keymap = useKeymap();
const uninstallKeymapListener = installKeymapListener();
const unregisterBoardSearch = keymap.registerShortcut({
  id: 'board-search',
  enabled: computed(() => !isView.value),
  priority: KEYMAP_PRIORITIES.board,
  handler: () => {
    boardSearchRef.value?.focus();
  },
});

const openTaskLive = useOpenTaskLive(ws);
const presence = useBoardPresence(ws, boardId);

const sidebarRef = ref<InstanceType<typeof TasksSidebar> | null>(null);

const PREDEFINED_VIEW_LABELS: Record<string, string> = {
  'my-tasks': 'My tasks',
  'recently-updated': 'Recently updated',
  'agent-activity': 'Agent activity',
};

function viewLabel(id: string): string {
  if (id in PREDEFINED_VIEW_LABELS) return PREDEFINED_VIEW_LABELS[id] as string;
  return taskViews.items.find((v) => v.id === id)?.name ?? 'View';
}

const breadcrumbs = computed(() => {
  if (isView.value && viewId.value !== null) {
    return ['Atlas', viewLabel(viewId.value), 'View'];
  }

  const name = boards.board?.name;
  return name !== undefined ? ['Atlas', name, 'Board'] : ['Atlas', 'Board'];
});

// The task opened on the board, shown through TaskDetailPane in the persisted
// view mode (sidebar dock or floating dialog). Full-screen mode navigates to the
// standalone /t/task/:id route instead of rendering inline.
const selectedReadableId = ref<string | null>(null);
let boardLoadOperation = 0;

const paneTask = computed(() => {
  if (selectedReadableId.value === null) return null;
  const open = tasks.openTask;
  return open && open.readable_id === selectedReadableId.value ? open : null;
});

const paneVisible = computed(() => paneTask.value !== null && ui.effectiveTaskViewMode !== 'full');
const boardDimmed = computed(() => paneVisible.value && ui.effectiveTaskViewMode === 'modal');

// The active board layout. Every view takes the same (ws, selected) contract and
// emits select/open, so the toolbar and detail pane wire identically across them.
const VIEW_COMPONENTS = {
  board: KanbanBoard,
  list: TaskListView,
  table: TaskTableView,
  calendar: TaskCalendarView,
  timeline: TaskTimelineView,
} as const;

const activeViewComponent = computed(() => VIEW_COMPONENTS[ui.taskView]);
const isBoardView = computed(() => ui.taskView === 'board');

const GROUP_OPTIONS: { id: TaskGroupBy; label: string }[] = [
  { id: 'status', label: 'Status' },
  { id: 'assignee', label: 'Assignee' },
  { id: 'priority', label: 'Priority' },
];

const groupLabel = computed(() => GROUP_OPTIONS.find((g) => g.id === ui.taskGroupBy)?.label ?? 'Status');

// Calendar, timeline and table render real due dates, which the bulk task summary
// omits; fetch the full task DTOs once when one of those layouts becomes active.
function ensureTaskDetails(view = ui.taskView): void {
  if ((view === 'calendar' || view === 'timeline' || view === 'table') && ws.value !== '') {
    void boards.loadTaskDetails(ws.value);
  }
}

watch(() => ui.taskView, ensureTaskDetails);

async function onSelect(readableId: string, mode?: TaskViewMode): Promise<void> {
  // A `mode` comes from the context menu's "Open as…" — a one-off presentation
  // that does not touch the saved default. A plain row click carries no mode, so
  // it drops any leftover override and falls back to the persisted preference.
  if (mode !== undefined) ui.openTaskInMode(mode);
  else ui.clearTaskViewModeOverride();

  // Full screen has no inline pane; open the standalone route instead. On mobile
  // the inline dock/dialog is too cramped, so a tapped card always opens full.
  if (isMobile.value || ui.effectiveTaskViewMode === 'full') {
    openTask(readableId);
    return;
  }

  selectedReadableId.value = readableId;
  await Promise.all([
    tasks.loadTask(ws.value, readableId),
    detail.loadAll(ws.value, readableId),
    workspace.loadMembers(ws.value),
  ]);
}

function closePane(): void {
  selectedReadableId.value = null;
  ui.clearTaskViewModeOverride();
}

const unregisterTaskEscape = keymap.registerShortcut({
  id: 'escape',
  enabled: paneVisible,
  priority: KEYMAP_PRIORITIES.task,
  handler: () => {
    closePane();
  },
});

onBeforeUnmount(() => {
  boardLoadOperation += 1;
  boards.cancelBoardLoad();
  unregisterBoardSearch();
  unregisterTaskEscape();
  uninstallKeymapListener();
});

function expandToFull(): void {
  const readableId = selectedReadableId.value;
  if (readableId === null) return;
  ui.setTaskViewMode('full');
  openTask(readableId);
}

function openTask(readableId: string): void {
  void router.push({ name: 'task-detail', params: { readableId } });
}

async function loadBoard(): Promise<void> {
  const operation = ++boardLoadOperation;
  if (ws.value === '') return;

  selectedReadableId.value = null;

  if (boardId.value === null) {
    // No board is selected (e.g. a freshly entered or empty workspace). Drop any
    // load error left over from a previous board so the empty state shows instead
    // of a stale "Couldn't load board" panel.
    boards.loadError = null;
    await resolveDefaultBoard();
    return;
  }

  const targetWorkspace = ws.value;
  const targetBoardId = boardId.value;
  const [isCurrentLoad] = await Promise.all([
    boards.loadBoardContents(targetWorkspace, targetBoardId),
    workspace.loadMembers(targetWorkspace),
  ]);
  if (
    operation !== boardLoadOperation ||
    !isCurrentLoad ||
    isView.value ||
    ws.value !== targetWorkspace ||
    boardId.value !== targetBoardId
  ) {
    return;
  }

  if (boards.loadError !== null) {
    if (boards.loadErrorStatus === 404) {
      lastViewed.clearIfMatches(targetWorkspace, { name: 'tasks', params: { boardId: targetBoardId } });
    }
    return;
  }

  const savedView = uiState.boardViewFor(targetBoardId);
  ui.setTaskView(savedView ?? 'board');

  ensureTaskDetails();
  await openFromQuery();
}

async function resolveDefaultBoard(): Promise<void> {
  if (workspace.projects.length === 0) {
    await workspace.loadProjects(ws.value);
  }

  const project = workspace.projects[0];
  if (project === undefined) return;

  await boards.loadBoards(ws.value, project.slug);

  const first = boards.boardSummaries[0];
  if (first !== undefined) {
    await router.replace({ name: 'tasks', params: { boardId: first.id } });
  }
}

// Returning from the full-screen route in sidebar/dialog mode carries the task
// to reopen as ?open=<readable-id>; consume it once and strip it from the URL.
async function openFromQuery(): Promise<void> {
  const open = route.query.open;
  if (typeof open !== 'string' || open.length === 0) return;

  await router.replace({ name: 'tasks', params: { boardId: boardId.value }, query: {} });
  await onSelect(open);
}

async function loadView(): Promise<void> {
  const operation = ++boardLoadOperation;
  const targetWorkspace = ws.value;
  const vid = viewId.value;
  if (targetWorkspace === '' || vid === null) return;

  boards.cancelBoardLoad();
  selectedReadableId.value = null;

  let customView = taskViews.items.find((view) => view.id === vid);
  if (PREDEFINED_VIEW_LABELS[vid] === undefined) {
    const metadataLoaded = await taskViews.load(targetWorkspace);
    if (
      !metadataLoaded ||
      operation !== boardLoadOperation ||
      !isView.value ||
      ws.value !== targetWorkspace ||
      viewId.value !== vid
    ) {
      return;
    }

    customView = taskViews.items.find((view) => view.id === vid);
    if (customView === undefined) return;
  }

  const params = paramsForView(vid, customView?.filters);
  const isCurrentLoad = await workspaceTasks.load(targetWorkspace, params);
  if (
    !isCurrentLoad ||
    operation !== boardLoadOperation ||
    !isView.value ||
    ws.value !== targetWorkspace ||
    viewId.value !== vid
  ) {
    return;
  }
}

// Reloads whichever surface is active — the kanban board or a task view — used
// to fully resynchronize after the live stream reconnects or on a board delete.
function reloadActive(): void {
  if (isView.value) void loadView();
  else void loadBoard();
}

// The column a task currently sits in on the loaded board, by task UUID.
function columnOfTask(taskId: string): string | undefined {
  for (const col of boards.columns) {
    if (boards.tasksByColumn(col.id).some((t) => t.id === taskId)) return col.id;
  }
  return undefined;
}

// Reflects a remote move using the existing optimistic-move primitive (fields
// are unchanged, so no refetch). A no-op when the card is already in the target
// column (an echo of this client's own move). When the task moved onto this
// board from elsewhere we don't hold its card yet, so fetch it.
function applyRemoteMove(taskId: string, toColumnId: string): void {
  const current = columnOfTask(taskId);
  if (current === toColumnId) return;

  if (current !== undefined) {
    boards.applyOptimisticMove(taskId, toColumnId, boards.tasksByColumn(toColumnId).length);
  } else if (boards.columns.some((c) => c.id === toColumnId)) {
    void boards.upsertTaskById(ws.value, taskId);
  }
}

// Applies another actor's change to the board and, when it targets the open
// task, to the detail pane. Board mutations run only in board mode; open-task
// refreshes apply in either mode since the pane can float over a task view too.
function onLiveEvent(evt: LiveUpdateEvent): void {
  const taskId = eventString(evt.data, 'task_id');
  const onCurrentBoard = !isView.value && evt.envelope.board_id === boardId.value;

  switch (evt.type) {
    case EVENT_TYPE.TASK_CREATED:
      if (taskId !== undefined && onCurrentBoard) void boards.upsertTaskById(ws.value, taskId);
      break;

    case EVENT_TYPE.TASK_UPDATED:
      if (taskId === undefined) break;
      if (onCurrentBoard) void boards.upsertTaskById(ws.value, taskId);
      openTaskLive.apply(evt.type, taskId);
      break;

    case EVENT_TYPE.TASK_MOVED: {
      if (taskId === undefined) break;
      const toColumn = eventString(evt.data, 'to_column_id');
      if (!isView.value && toColumn !== undefined) applyRemoteMove(taskId, toColumn);
      openTaskLive.apply(evt.type, taskId);
      break;
    }

    case EVENT_TYPE.TASK_DELETED:
      if (taskId === undefined) break;
      if (!isView.value) boards.removeTaskById(taskId);
      if (openTaskLive.apply(evt.type, taskId) === 'deleted') closePane();
      break;

    case EVENT_TYPE.COLUMN_CREATED:
    case EVENT_TYPE.COLUMN_DELETED:
      if (onCurrentBoard && boardId.value !== null) void boards.loadColumns(ws.value, boardId.value);
      break;

    case EVENT_TYPE.BOARD_DELETED:
      if (onCurrentBoard) reloadActive();
      break;

    case PRESENCE_UPDATED:
      presence.apply(evt.envelope);
      break;

    default:
      break;
  }
}

useLiveUpdates(ws, { onEvent: onLiveEvent, onResync: reloadActive });

// A board id is only valid within its own workspace. When the active workspace
// changes, clear board-scoped state so a board (or a stale load error) from the
// previous workspace never bleeds into the next one. Registered before the load
// watcher so the reset runs first on a workspace switch.
watch(ws, () => {
  boards.reset();
});

watch(
  [boardId, viewId, ws],
  () => {
    if (isView.value) {
      void loadView();
    } else {
      void loadBoard();
    }
  },
  { immediate: true },
);
</script>

<template>
  <AppShell sidebar-title="Tasks" sidebar-icon="square-kanban" :mobile-detail="true">
    <template #sidebar-actions>
      <button type="button" class="atl-gbtn" title="Search ⌘K" aria-label="Search" @click="ui.openPalette()">
        <Icon name="search" :size="14" />
      </button>
      <button
        type="button"
        class="atl-gbtn"
        title="Collapse sidebar"
        aria-label="Collapse sidebar"
        @click="ui.toggleSidebar()"
      >
        <Icon name="panel-left" :size="13" />
      </button>
    </template>

    <template #sidebar>
      <TasksSidebar ref="sidebarRef" />
    </template>

    <template #sidebar-footer>
      <button
        type="button"
        class="atl-gbtn"
        style="width: 100%; justify-content: flex-start; height: 26px; gap: 7px; color: var(--c-foreground);"
        @click="sidebarRef?.openNewProject()"
      >
        <Icon name="plus" :size="14" />
        New project
      </button>
    </template>

    <div
      v-if="isMobile"
      class="flex items-center"
      style="height: 44px; flex: 0 0 44px; padding: 0 10px; gap: 8px; border-bottom: 1px solid var(--c-border);"
    >
      <span class="flex-1 truncate" style="font-size: var(--fs-lg); font-weight: var(--fw-bold); color: var(--c-foreground);">
        {{ boards.board?.name ?? 'Board' }}
      </span>
      <button type="button" class="atl-gbtn" title="Search ⌘K" aria-label="Search" @click="ui.openPalette()">
        <Icon name="search" :size="15" />
      </button>
    </div>

    <EditorToolbar v-else :breadcrumbs="breadcrumbs" :dirty="false">
      <template #lead>
        <BoardViewMenu v-if="!isView" />
        <Popover v-if="!isView && !isBoardView" placement="bottom-start" width="180px">
          <template #trigger="{ open, toggle }">
            <button
              type="button"
              class="atl-gbtn"
              :title="`Group by: ${groupLabel}`"
              aria-haspopup="menu"
              :aria-expanded="open"
              @click="toggle"
            >
              <Icon name="user" :size="14" />
              Group: {{ groupLabel }}
            </button>
          </template>
          <template #default="{ close }">
            <div style="padding: 5px 0;">
              <div
                v-for="opt in GROUP_OPTIONS"
                :key="opt.id"
                class="atl-vmi"
                :class="{ on: opt.id === ui.taskGroupBy }"
                role="menuitem"
                @click="ui.setTaskGroupBy(opt.id), close()"
              >
                <span style="flex: 1;">{{ opt.label }}</span>
                <Icon
                  v-if="opt.id === ui.taskGroupBy"
                  name="check"
                  :size="13"
                  style="color: var(--c-primary); flex: 0 0 auto;"
                />
              </div>
            </div>
          </template>
        </Popover>
      </template>

      <PresenceAvatars v-if="!isView" :actors="presence.actors" />
      <div v-if="!isView" class="atl-board-search">
        <Icon name="search" :size="13" class="atl-board-search-icon" />
        <input
          ref="boardSearchRef"
          :value="ui.taskFilterText"
          type="text"
          class="atl-board-search-input"
          placeholder="Search tasks…"
          aria-label="Search tasks"
          @input="ui.setTaskFilterText(($event.target as HTMLInputElement).value)"
          @keydown.esc="onSearchKeydownEsc"
        />
        <button
          v-if="ui.taskFilterText !== ''"
          type="button"
          class="atl-board-search-clear"
          title="Clear search"
          aria-label="Clear search"
          @click="clearBoardSearch"
        >
          <Icon name="x" :size="12" />
        </button>
      </div>
      <Popover v-if="!isView" placement="bottom-end">
        <template #trigger="{ open, toggle }">
          <button
            type="button"
            class="atl-gbtn"
            :class="{ on: ui.hasActiveFilter }"
            title="Filter"
            aria-label="Filter"
            aria-haspopup="dialog"
            :aria-expanded="open"
            @click="toggle"
          >
            <span class="atl-filter-icon" style="position: relative; display: inline-flex;">
              <Icon name="filter" :size="14" />
              <span
                v-if="ui.hasActiveFilter"
                aria-hidden="true"
                style="position: absolute; top: -2px; right: -3px; width: 6px; height: 6px; border-radius: var(--r-full); background: var(--c-primary);"
              />
            </span>
            Filter
          </button>
        </template>
        <template #default>
          <TaskFilterPanel />
        </template>
      </Popover>
      <button
        type="button"
        class="atl-gbtn"
        title="Command palette ⌘K"
        aria-label="Command palette"
        @click="ui.openPalette()"
      >
        <Icon name="command" :size="14" />
      </button>
    </EditorToolbar>

    <template v-if="isView">
      <ErrorState
        v-if="workspaceTasks.error"
        title="Couldn't load view"
        :hint="workspaceTasks.error"
        @retry="loadView"
      />
      <LoadingState v-else-if="workspaceTasks.loading" label="Loading…" />
      <div v-else class="flex flex-1 min-h-0" style="position: relative;">
        <TaskViewListView
          :ws="ws"
          :tasks="workspaceTasks.tasks"
          :selected-readable-id="selectedReadableId"
          :class="{ 'atl-board-dimmed': boardDimmed }"
          @select="onSelect"
          @open="openTask"
        />
        <TaskDetailPane
          v-if="paneVisible && paneTask"
          :task="paneTask"
          :ws="ws"
          @close="closePane"
          @expand="expandToFull"
          @open-subtask="onSelect"
        />
      </div>
    </template>

    <template v-else>
      <LoadingState v-if="boards.loading" label="Loading…" />
      <EmptyState
        v-else-if="boardNotFound"
        title="Board not found"
        hint="This board no longer exists. Pick another board from the sidebar."
        icon="square-kanban"
      />
      <ErrorState
        v-else-if="boards.loadError"
        title="Couldn't load board"
        :hint="boards.loadError"
        @retry="loadBoard"
      />
      <EmptyState
        v-else-if="boardId === null"
        title="No board selected"
        hint="Pick a board from the sidebar to see its tasks"
        icon="square-kanban"
      />
      <div v-else class="flex flex-1 min-h-0" style="position: relative;">
        <component
          :is="activeViewComponent"
          :ws="ws"
          :selected-readable-id="selectedReadableId"
          :class="{ 'atl-board-dimmed': boardDimmed }"
          @select="onSelect"
          @open="openTask"
        />
        <TaskDetailPane
          v-if="paneVisible && paneTask"
          :task="paneTask"
          :ws="ws"
          @close="closePane"
          @expand="expandToFull"
          @open-subtask="onSelect"
        />
      </div>
    </template>
  </AppShell>
</template>

<style scoped>
.atl-board-dimmed {
  filter: saturate(0.7) brightness(0.82);
  transition: filter 0.2s;
}

.atl-board-search {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 8px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  background: var(--c-background);
  color: var(--c-muted);
}

.atl-board-search:focus-within {
  border-color: var(--c-primary);
}

.atl-board-search-icon {
  flex: 0 0 auto;
}

.atl-board-search-input {
  width: 150px;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  color: var(--c-foreground);
  font-family: var(--font-ui);
  font-size: var(--fs-sm);
}

.atl-board-search-input::placeholder {
  color: var(--c-muted);
}

.atl-board-search-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  padding: 0;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
  flex: 0 0 auto;
}

.atl-board-search-clear:hover {
  color: var(--c-foreground);
  background: var(--c-raised);
}
</style>
