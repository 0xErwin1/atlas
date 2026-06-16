<script setup lang="ts">
import { computed, onMounted, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import Icon from '@/components/ui/Icon.vue';
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
  <div v-if="activeProject">
    <div
      style="
        padding: 8px 8px 4px;
        font-size: var(--fs-xs);
        font-weight: var(--fw-semibold);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        color: var(--c-muted);
      "
    >
      {{ activeProject.name }}
    </div>

    <button
      v-for="b in boards.boardSummaries"
      :key="b.id"
      type="button"
      class="flex items-center w-full text-left cursor-pointer"
      :style="{
        gap: '8px',
        height: 'var(--h-compact)',
        padding: '0 8px',
        border: 'none',
        background: activeBoardId === b.id ? 'var(--c-list-active)' : 'transparent',
        color: activeBoardId === b.id ? 'var(--c-foreground)' : 'var(--c-muted)',
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-sm)',
        fontWeight: activeBoardId === b.id ? 'var(--fw-semibold)' : 'var(--fw-normal)',
      }"
      @click="openBoard(b.id)"
    >
      <Icon
        name="columns-3"
        :size="13"
        :style="{ color: activeBoardId === b.id ? 'var(--c-primary)' : 'var(--c-muted)' }"
      />
      <span class="flex-1 min-w-0 truncate">{{ b.name }}</span>
    </button>

    <p
      v-if="boards.boardSummaries.length === 0"
      style="padding: 8px; font-size: var(--fs-sm); color: var(--c-muted);"
    >
      No boards in this project.
    </p>
  </div>

  <p
    v-else
    style="padding: 8px; font-size: var(--fs-sm); color: var(--c-muted);"
  >
    No project selected.
  </p>
</template>
