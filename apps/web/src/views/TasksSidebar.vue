<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import Row from '@/components/ui/Row.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { useInlineEdit } from '@/composables/useInlineEdit';
import { useBoardsStore } from '@/stores/boards';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const route = useRoute();
const router = useRouter();
const workspace = useWorkspaceStore();
const boards = useBoardsStore();
const ui = useUiStore();

const activeBoardId = computed(() => {
  const id = route.params.boardId;
  return typeof id === 'string' ? id : null;
});

const activeProject = computed(() => workspace.projects[0] ?? null);

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

async function loadBoards(): Promise<void> {
  const wsSlug = workspace.activeWorkspaceSlug;
  if (wsSlug === null) {
    await workspace.loadProjects('');
    return;
  }

  if (workspace.projects.length === 0) {
    await workspace.loadProjects(wsSlug);
  }

  const project = activeProject.value;
  if (project === null) {
    return;
  }

  await boards.loadBoards(wsSlug, project.slug);
}

function openBoard(boardId: string): void {
  void router.push({ name: 'tasks', params: { boardId } });
}

// Shared sidebar context-menu + inline-edit logic (same composables as the notes sidebar).
const { open: menuOpen, x: menuX, y: menuY, openAt, close: closeMenu } = useContextMenu();

type EditCtx = { kind: 'new-board' } | { kind: 'rename-board'; boardId: string };

const {
  active: editActive,
  value: editValue,
  inputRef,
  start: startEdit,
  commit: commitEdit,
  onKeydown: onEditKeydown,
} = useInlineEdit<EditCtx>(async (name, ctx) => {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  if (ctx.kind === 'new-board') {
    const id = await boards.createBoard(ws.value, project.slug, name);
    if (id !== null) openBoard(id);
    else if (boards.error) ui.showBanner(boards.error, 'error');
  } else {
    const ok = await boards.renameBoard(ws.value, project.slug, ctx.boardId, name);
    if (!ok && boards.error) ui.showBanner(boards.error, 'error');
  }
});

async function removeBoard(boardId: string): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  const ok = await boards.removeBoard(ws.value, project.slug, boardId);
  if (!ok && boards.error) ui.showBanner(boards.error, 'error');
}

type MenuTarget = { kind: 'project' } | { kind: 'board'; boardId: string; name: string };
const menuTarget = ref<MenuTarget>({ kind: 'project' });

const projectMenuItems = computed<MenuItem[]>(() => [
  { label: 'New board', icon: 'plus', action: () => startEdit({ kind: 'new-board' }) },
]);

const boardMenuItems = computed<MenuItem[]>(() => {
  const state = menuTarget.value;
  if (state.kind !== 'board') return [];
  const { boardId, name } = state;
  return [
    { header: true, label: name },
    { label: 'Open', icon: 'external-link', kbd: ['↵'], action: () => openBoard(boardId) },
    { sep: true },
    {
      label: 'Rename',
      icon: 'pencil',
      kbd: ['F2'],
      action: () => startEdit({ kind: 'rename-board', boardId }, name, true),
    },
    { sep: true },
    { label: 'Delete', icon: 'trash', kbd: ['⌫'], danger: true, action: () => removeBoard(boardId) },
  ];
});

const activeMenuItems = computed<MenuItem[]>(() =>
  menuTarget.value.kind === 'board' ? boardMenuItems.value : projectMenuItems.value,
);

function openProjectMenu(event: MouseEvent): void {
  menuTarget.value = { kind: 'project' };
  openAt(event);
}

function openBoardMenu(event: MouseEvent, boardId: string, name: string): void {
  menuTarget.value = { kind: 'board', boardId, name };
  openAt(event);
}

onMounted(loadBoards);
watch(() => workspace.activeWorkspaceSlug, loadBoards);
</script>

<template>
  <template v-if="activeProject">
    <div class="tasks-sidebar-header" @contextmenu.prevent="openProjectMenu">
      <SectionLabel>Projects</SectionLabel>
      <button
        type="button"
        class="tasks-sidebar-add"
        title="New board"
        aria-label="New board"
        @click.stop="openProjectMenu"
      >
        <Icon name="more-horizontal" :size="14" />
      </button>
    </div>

    <Row
      :label="activeProject.name"
      icon="folder-open"
      :chevron="true"
      :open="true"
      menu
      @menu="openProjectMenu"
      @contextmenu.prevent.stop="openProjectMenu"
    />

    <template v-for="b in boards.boardSummaries" :key="b.id">
      <div
        v-if="editActive?.kind === 'rename-board' && editActive.boardId === b.id"
        style="display: flex; align-items: center; gap: 6px; padding: 3px 8px 3px 22px;"
      >
        <Icon name="columns-3" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
        <input
          ref="inputRef"
          v-model="editValue"
          type="text"
          placeholder="Board name…"
          class="tasks-inline-input"
          @keydown="onEditKeydown"
          @blur="commitEdit"
        />
      </div>
      <Row
        v-else
        :label="b.name"
        icon="columns-3"
        :depth="1"
        :active="activeBoardId === b.id"
        menu
        @click="openBoard(b.id)"
        @menu="(event: MouseEvent) => openBoardMenu(event, b.id, b.name)"
        @contextmenu.prevent.stop="(event: MouseEvent) => openBoardMenu(event, b.id, b.name)"
      />
    </template>

    <div
      v-if="editActive?.kind === 'new-board'"
      style="display: flex; align-items: center; gap: 6px; padding: 3px 8px 3px 22px;"
    >
      <Icon name="columns-3" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
      <input
        ref="inputRef"
        v-model="editValue"
        type="text"
        placeholder="Board name…"
        class="tasks-inline-input"
        @keydown="onEditKeydown"
        @blur="commitEdit"
      />
    </div>

    <p
      v-if="boards.boardSummaries.length === 0 && editActive === null"
      style="padding: 8px 8px 8px 22px; font-size: var(--fs-sm); color: var(--c-muted);"
    >
      No boards in this project.
    </p>

    <ContextMenu
      :open="menuOpen"
      :x="menuX"
      :y="menuY"
      :items="activeMenuItems"
      @close="closeMenu"
    />
  </template>

  <p
    v-else
    style="padding: 8px; font-size: var(--fs-sm); color: var(--c-muted);"
  >
    No project selected.
  </p>
</template>

<style scoped>
.tasks-sidebar-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.tasks-sidebar-add {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  margin-right: 6px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--c-muted);
  border-radius: var(--r-sm);
  cursor: pointer;
  opacity: 0;
}

.tasks-sidebar-header:hover .tasks-sidebar-add {
  opacity: 1;
}

.tasks-sidebar-add:hover {
  background: var(--c-raised);
  color: var(--c-foreground);
}

.tasks-inline-input {
  flex: 1;
  height: 28px;
  padding: 0 6px;
  background: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  font-size: var(--fs-sm);
  font-family: var(--font-mono);
  color: var(--c-foreground);
  outline: none;
}
</style>
