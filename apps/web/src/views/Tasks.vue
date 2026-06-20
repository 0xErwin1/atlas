<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import EditorToolbar from '@/components/shell/EditorToolbar.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import BoardViewMenu from '@/components/tareas/BoardViewMenu.vue';
import KanbanBoard from '@/components/tareas/KanbanBoard.vue';
import TaskDetailPane from '@/components/tareas/TaskDetailPane.vue';
import Icon from '@/components/ui/Icon.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import { useBoardsStore } from '@/stores/boards';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useTasksStore } from '@/stores/tasks';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';
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
const { isMobile } = useBreakpoint();

const boardId = computed(() => {
  const id = route.params.boardId;
  return typeof id === 'string' ? id : null;
});

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const sidebarRef = ref<InstanceType<typeof TasksSidebar> | null>(null);

const breadcrumbs = computed(() => {
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
  await Promise.all([boards.loadColumns(ws.value, boardId.value), boards.loadTasks(ws.value, boardId.value)]);

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

watch([boardId, ws], loadBoard, { immediate: true });
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
        <BoardViewMenu />
      </template>

      <button
        type="button"
        class="atl-gbtn"
        title="Filter"
        aria-label="Filter"
        @click="ui.showBanner('Filtering is coming soon', 'info')"
      >
        <Icon name="filter" :size="14" />
        Filter
      </button>
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

    <ErrorState
      v-if="boards.error"
      title="Couldn’t load board"
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
      <KanbanBoard
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
      />
    </div>
  </AppShell>
</template>

<style scoped>
.atl-board-dimmed {
  filter: saturate(0.7) brightness(0.82);
  transition: filter 0.2s;
}
</style>
