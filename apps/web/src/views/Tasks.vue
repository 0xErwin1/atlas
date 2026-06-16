<script setup lang="ts">
import { computed, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import EditorToolbar from '@/components/shell/EditorToolbar.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import KanbanBoard from '@/components/tareas/KanbanBoard.vue';
import { useBoardsStore } from '@/stores/boards';
import { useWorkspaceStore } from '@/stores/workspace';
import AppShell from '@/views/AppShell.vue';
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

const breadcrumbs = computed(() => ['Atlas', boards.board?.name ?? 'Board']);

async function loadBoard(): Promise<void> {
  if (boardId.value === null || ws.value === '') {
    return;
  }

  await boards.loadBoard(ws.value, boardId.value);
  await Promise.all([boards.loadColumns(ws.value, boardId.value), boards.loadTasks(ws.value, boardId.value)]);
}

function openTask(readableId: string): void {
  void router.push({ name: 'task-detail', params: { readableId } });
}

watch([boardId, ws], loadBoard, { immediate: true });
</script>

<template>
  <AppShell>
    <template #sidebar>
      <TasksSidebar />
    </template>

    <EditorToolbar :breadcrumbs="breadcrumbs" :dirty="false" />

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
