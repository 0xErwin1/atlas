<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import EditorToolbar from '@/components/shell/EditorToolbar.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import KanbanBoard from '@/components/tareas/KanbanBoard.vue';
import Icon from '@/components/ui/Icon.vue';
import { useBoardsStore } from '@/stores/boards';
import { useWorkspaceStore } from '@/stores/workspace';
import AppShell from '@/views/AppShell.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import TasksSidebar from '@/views/TasksSidebar.vue';

const route = useRoute();
const router = useRouter();
const workspace = useWorkspaceStore();
const boards = useBoardsStore();

const boardId = computed(() => {
  const id = route.params.boardId;
  return typeof id === 'string' ? id : null;
});

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const sidebarRef = ref<InstanceType<typeof TasksSidebar> | null>(null);

const breadcrumbs = computed(() => ['Atlas', boards.board?.name ?? 'Board']);

async function loadBoard(): Promise<void> {
  if (ws.value === '') return;

  // No board in the URL (e.g. the rail "Tasks" button): pick the project's first
  // board and redirect to it, mirroring how /n opens without a slug.
  if (boardId.value === null) {
    await resolveDefaultBoard();
    return;
  }

  await boards.loadBoard(ws.value, boardId.value);
  await Promise.all([boards.loadColumns(ws.value, boardId.value), boards.loadTasks(ws.value, boardId.value)]);
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

function openTask(readableId: string): void {
  void router.push({ name: 'task-detail', params: { readableId } });
}

watch([boardId, ws], loadBoard, { immediate: true });
</script>

<template>
  <AppShell sidebar-title="Tasks" sidebar-icon="square-kanban">
    <template #sidebar-actions>
      <button type="button" class="atl-gbtn" title="Filter" aria-label="Filter">
        <Icon name="search" :size="14" />
      </button>
      <button type="button" class="atl-gbtn" title="Collapse" aria-label="Collapse sidebar">
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

    <EditorToolbar :breadcrumbs="breadcrumbs" :dirty="false">
      <button type="button" class="atl-gbtn" title="Filter" aria-label="Filter">
        <Icon name="filter" :size="14" />
        Filter
      </button>
      <button type="button" class="atl-gbtn" title="Command palette ⌘K" aria-label="Command palette">
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
    <KanbanBoard v-else :ws="ws" @open="openTask" />
  </AppShell>
</template>
