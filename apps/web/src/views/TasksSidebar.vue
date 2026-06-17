<script setup lang="ts">
import { computed, onMounted, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import Row from '@/components/ui/Row.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import { useBoardsStore } from '@/stores/boards';
import { useWorkspaceStore } from '@/stores/workspace';

const route = useRoute();
const router = useRouter();
const workspace = useWorkspaceStore();
const boards = useBoardsStore();

const activeBoardId = computed(() => {
  const id = route.params.boardId;
  return typeof id === 'string' ? id : null;
});

const activeProject = computed(() => workspace.projects[0] ?? null);

async function loadBoards(): Promise<void> {
  const ws = workspace.activeWorkspaceSlug;
  if (ws === null) {
    await workspace.loadProjects('');
    return;
  }

  if (workspace.projects.length === 0) {
    await workspace.loadProjects(ws);
  }

  const project = activeProject.value;
  if (project === null) {
    return;
  }

  await boards.loadBoards(ws, project.slug);
}

function openBoard(boardId: string): void {
  void router.push({ name: 'tasks', params: { boardId } });
}

onMounted(loadBoards);
watch(() => workspace.activeWorkspaceSlug, loadBoards);
</script>

<template>
  <template v-if="activeProject">
    <SectionLabel>Projects</SectionLabel>

    <Row
      :label="activeProject.name"
      icon="folder-open"
      :chevron="true"
      :open="true"
    />

    <Row
      v-for="b in boards.boardSummaries"
      :key="b.id"
      :label="b.name"
      icon="columns-3"
      :depth="1"
      :active="activeBoardId === b.id"
      @click="openBoard(b.id)"
    />

    <p
      v-if="boards.boardSummaries.length === 0"
      style="padding: 8px 8px 8px 22px; font-size: var(--fs-sm); color: var(--c-muted);"
    >
      No boards in this project.
    </p>
  </template>

  <p
    v-else
    style="padding: 8px; font-size: var(--fs-sm); color: var(--c-muted);"
  >
    No project selected.
  </p>
</template>
