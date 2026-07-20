<script setup lang="ts">
import { computed, onBeforeUnmount, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import EmptyState from '@/components/states/EmptyState.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import TaskBody from '@/components/tareas/TaskBody.vue';
import TaskDetailHeader from '@/components/tareas/TaskDetailHeader.vue';
import TaskInspector from '@/components/tareas/TaskInspector.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import { installKeymapListener, useKeymap } from '@/composables/useKeymap';
import { type LiveUpdateEvent, useLiveUpdates } from '@/composables/useLiveUpdates';
import { useOpenTaskLive } from '@/composables/useOpenTaskLive';
import { useResizablePanel } from '@/composables/useResizablePanel';
import { safeBackOrBoard } from '@/composables/useTaskEscapeNavigation';
import { EVENT_TYPE, eventString } from '@/lib/eventTypes';
import { KEYMAP_PRIORITIES } from '@/lib/keymap';
import { useBoardsStore } from '@/stores/boards';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useTasksStore } from '@/stores/tasks';
import { type TaskViewMode, useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';
import AppShell from '@/views/AppShell.vue';
import NotesSidebar from '@/views/NotesSidebar.vue';

const route = useRoute();
const router = useRouter();
const workspace = useWorkspaceStore();
const tasks = useTasksStore();
const detail = useTaskDetailStore();
const boards = useBoardsStore();
const ui = useUiStore();
const lastViewed = useLastViewedStore();
const { isMobile } = useBreakpoint();

// A restored task that no longer exists loads as a 404: show an empty state, not
// an error, and stop restoring the dead entry on the next workspace switch.
const notFound = computed(() => tasks.error !== null && tasks.errorStatus === 404);

const readableId = computed(() => {
  const id = route.params.readableId;
  return typeof id === 'string' ? id : null;
});

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const openTaskLive = useOpenTaskLive(ws);

const task = computed(() => tasks.openTask);
let loadSeq = 0;

const keymap = useKeymap();
const uninstallKeymapListener = installKeymapListener();
function navigateBack(): false | undefined {
  const currentTask = task.value;
  if (currentTask === null) return false;
  void safeBackOrBoard({ task: currentTask, router, route });
}

const unregisterTaskEscape = keymap.registerShortcut({
  id: 'escape',
  enabled: computed(() => task.value !== null),
  priority: KEYMAP_PRIORITIES.task,
  handler: navigateBack,
});

onBeforeUnmount(() => {
  unregisterTaskEscape();
  uninstallKeymapListener();
});

// Resizable inspector (activity + comments): the user drags the divider to make
// the panel as wide as they need; the width persists across tasks and reloads.
const { width: inspectorWidth, startResize } = useResizablePanel({
  storageKey: 'atlas:task-inspector-width',
  min: 300,
  max: 680,
  initial: 400,
});

const breadcrumbs = computed(() => [
  'Atlas',
  boards.board?.name ?? 'Tasks',
  task.value?.readable_id ?? 'Task',
]);

const shareLabel = computed(() => `${task.value?.readable_id ?? 'Task'} · task`);

async function load(): Promise<void> {
  if (readableId.value === null || ws.value === '') {
    return;
  }

  const seq = ++loadSeq;
  const target = { readableId: readableId.value, ws: ws.value };
  const workspaceId = workspace.workspaceIdForSlug(target.ws);
  detail.clear();

  if (workspaceId === null) {
    await tasks.loadTask(target.ws, target.readableId);
  } else {
    await tasks.loadTask(target.ws, target.readableId, workspaceId);
  }

  const superseded = seq !== loadSeq || readableId.value !== target.readableId || ws.value !== target.ws;
  if (superseded) return;

  if (tasks.error !== null && tasks.openTask?.readable_id !== target.readableId) {
    if (tasks.errorStatus === 404) {
      lastViewed.clearIfMatches(target.ws, {
        name: 'task-detail',
        params: { readableId: target.readableId },
      });
    }
    return;
  }

  if (tasks.openTask?.readable_id !== target.readableId) {
    return;
  }

  const openTask = tasks.openTask;
  const boardId = openTask?.board_id;
  await Promise.all([
    workspaceId === null
      ? detail.loadAll(target.ws, target.readableId, undefined, openTask.id)
      : detail.loadAll(target.ws, target.readableId, workspaceId, openTask.id),
    workspace.loadMembers(target.ws),
    boardId === undefined
      ? Promise.resolve()
      : Promise.all([boards.loadBoard(target.ws, boardId), boards.loadColumns(target.ws, boardId)]),
  ]);
}

// Already on the full-screen route, so a sub-task stays full screen — navigating
// swaps the route param and reloads this view for the child.
function openSubtask(readableId: string): void {
  void router.push({ name: 'task-detail', params: { readableId } });
}

async function onDeleteTask(): Promise<void> {
  const id = task.value?.readable_id;
  if (id === undefined || ws.value === '') return;

  const ok = await boards.deleteTask(ws.value, id);
  if (ok) {
    await tasks.retractTask(id);
    detail.clear();
    ui.showBanner('Task deleted', 'success');
    backToBoard();
  } else {
    ui.showBanner(boards.error ?? 'Failed to delete task', 'error');
  }
}

function backToBoard(query: Record<string, string> = {}): void {
  const boardId = task.value?.board_id;
  if (boardId === undefined) {
    void router.push({ name: 'tasks' });
    return;
  }
  void router.push({ name: 'tasks', params: { boardId }, query });
}

// Switching away from full screen returns to the board, which reopens this task
// in the freshly chosen dock/dialog mode via the ?open marker.
function onChangeMode(mode: TaskViewMode): void {
  if (mode === 'full') return;
  const id = task.value?.readable_id;
  if (id === undefined) return;
  backToBoard({ open: id });
}

// Reacts to another actor's change to the open task: an update or move reloads
// the detail and its collections; a delete returns to the board (the task is
// gone, so there is nothing to show here).
function onLiveEvent(evt: LiveUpdateEvent): void {
  const taskId = eventString(evt.data, 'task_id');
  if (taskId === undefined) return;

  switch (evt.type) {
    case EVENT_TYPE.TASK_UPDATED:
    case EVENT_TYPE.TASK_MOVED:
    case EVENT_TYPE.TASK_DELETED:
      if (openTaskLive.apply(evt.type, taskId) === 'deleted') backToBoard();
      break;

    default:
      break;
  }
}

useLiveUpdates(ws, { onEvent: onLiveEvent, onResync: () => void load() });

watch([readableId, ws], load, { immediate: true });
</script>

<template>
  <AppShell sidebar-title="Tasks" sidebar-icon="square-kanban" :mobile-detail="true">
    <template #sidebar>
      <NotesSidebar />
    </template>

    <TaskDetailHeader
      v-if="task"
      :readable-id="task.readable_id"
      :share-label="shareLabel"
      :breadcrumbs="breadcrumbs"
      show-back
      :show-close="false"
      :show-inspector-toggle="!isMobile"
      :inspector-open="ui.taskInspectorOpen"
      @back="navigateBack"
      @change="onChangeMode"
      @toggle-inspector="ui.toggleTaskInspector()"
      @delete="onDeleteTask"
    />

    <div v-if="task" class="flex flex-1 min-h-0">
      <div class="flex-1 overflow-y-auto" :style="isMobile ? 'padding: 16px;' : 'padding: 24px 40px;'">
        <TaskBody
          :task="task"
          :ws="ws"
          layout="wide"
          :show-secondary="isMobile"
          @open-subtask="openSubtask"
        />
      </div>
      <template v-if="!isMobile && ui.taskInspectorOpen">
        <div
          class="atl-inspector-resizer"
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize panel"
          @mousedown.prevent="startResize"
        />
        <TaskInspector :task="task" :ws="ws" :width="inspectorWidth" />
      </template>
    </div>
    <div v-else class="flex-1 overflow-y-auto" style="padding: 24px 40px;">
      <EmptyState
        v-if="notFound"
        title="Task not found"
        hint="This task no longer exists. Pick another task from the board."
        icon="square-kanban"
      />
      <ErrorState
        v-else-if="tasks.error"
        title="Couldn’t load task"
        :hint="tasks.error"
        @retry="load"
      />
      <LoadingState v-else label="Loading task…" />
    </div>
  </AppShell>
</template>

<style scoped>
.atl-inspector-resizer {
  flex: 0 0 5px;
  cursor: col-resize;
  background: transparent;
  border-left: 1px solid var(--c-border);
  transition: background 0.12s;
}

.atl-inspector-resizer:hover {
  background: var(--c-primary);
}
</style>
