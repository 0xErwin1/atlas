<script setup lang="ts">
import { computed, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import TaskBody from '@/components/tareas/TaskBody.vue';
import TaskDetailHeader from '@/components/tareas/TaskDetailHeader.vue';
import TaskInspector from '@/components/tareas/TaskInspector.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import { useResizablePanel } from '@/composables/useResizablePanel';
import { useBoardsStore } from '@/stores/boards';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useTasksStore } from '@/stores/tasks';
import { type TaskViewMode, useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';
import AppShell from '@/views/AppShell.vue';
import TasksSidebar from '@/views/TasksSidebar.vue';

const route = useRoute();
const router = useRouter();
const workspace = useWorkspaceStore();
const tasks = useTasksStore();
const detail = useTaskDetailStore();
const boards = useBoardsStore();
const ui = useUiStore();
const { isMobile } = useBreakpoint();

const readableId = computed(() => {
  const id = route.params.readableId;
  return typeof id === 'string' ? id : null;
});

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const task = computed(() => tasks.openTask);

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

  await tasks.loadTask(ws.value, readableId.value);

  const boardId = tasks.openTask?.board_id;
  await Promise.all([
    detail.loadAll(ws.value, readableId.value),
    workspace.loadMembers(ws.value),
    boardId === undefined
      ? Promise.resolve()
      : Promise.all([boards.loadBoard(ws.value, boardId), boards.loadColumns(ws.value, boardId)]),
  ]);
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

watch([readableId, ws], load, { immediate: true });
</script>

<template>
  <AppShell sidebar-title="Tasks" sidebar-icon="square-kanban" :mobile-detail="true">
    <template #sidebar>
      <TasksSidebar />
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
      @back="backToBoard()"
      @change="onChangeMode"
      @toggle-inspector="ui.toggleTaskInspector()"
    />

    <div v-if="task" class="flex flex-1 min-h-0">
      <div class="flex-1 overflow-y-auto" :style="isMobile ? 'padding: 16px;' : 'padding: 24px 40px;'">
        <TaskBody :task="task" :ws="ws" layout="wide" :show-secondary="isMobile" />
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
      <ErrorState
        v-if="tasks.error"
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
