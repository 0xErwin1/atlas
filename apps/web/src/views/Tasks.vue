<script setup lang="ts">
import { computed, ref, watch } from 'vue';
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
import { useBreakpoint } from '@/composables/useBreakpoint';
import { useBoardsStore } from '@/stores/boards';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useTasksStore } from '@/stores/tasks';
import { useTaskViewsStore } from '@/stores/taskViews';
import type { TaskGroupBy } from '@/stores/ui';
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
const { isMobile } = useBreakpoint();

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

const paneTask = computed(() => {
  if (selectedReadableId.value === null) return null;
  const open = tasks.openTask;
  return open && open.readable_id === selectedReadableId.value ? open : null;
});

const paneVisible = computed(() => paneTask.value !== null && ui.taskViewMode !== 'full');
const boardDimmed = computed(() => paneVisible.value && ui.taskViewMode === 'modal');

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

async function onSelect(readableId: string): Promise<void> {
  // The persisted preference may be full screen — then the board has no inline
  // pane; open the standalone route instead. On mobile the inline dock/dialog is
  // too cramped, so a tapped card always opens the full-screen route.
  if (isMobile.value || ui.taskViewMode === 'full') {
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
}

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
  if (ws.value === '') return;

  selectedReadableId.value = null;

  if (boardId.value === null) {
    await resolveDefaultBoard();
    return;
  }

  await boards.loadBoard(ws.value, boardId.value);
  await Promise.all([
    boards.loadColumns(ws.value, boardId.value),
    boards.loadTasks(ws.value, boardId.value),
    workspace.loadMembers(ws.value),
  ]);

  const savedView = uiState.boardViewFor(boardId.value);
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
  const vid = viewId.value;
  if (ws.value === '' || vid === null) return;

  selectedReadableId.value = null;

  const customView = taskViews.items.find((v) => v.id === vid);
  const params = paramsForView(vid, customView?.filters);
  await workspaceTasks.load(ws.value, params);
}

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

      <Popover v-if="!isView" placement="bottom-start">
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
          @navigate="openTask"
        />
      </div>
    </template>

    <template v-else>
      <ErrorState
        v-if="boards.error"
        title="Couldn't load board"
        :hint="boards.error"
        @retry="loadBoard"
      />
      <LoadingState v-else-if="boards.loading" label="Loading…" />
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
          @navigate="openTask"
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
</style>
