<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import Row from '@/components/ui/Row.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { useInlineEdit } from '@/composables/useInlineEdit';
import { useBoardsStore } from '@/stores/boards';
import { useDocumentsStore } from '@/stores/documents';
import { useFoldersStore } from '@/stores/folders';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const route = useRoute();
const router = useRouter();
const workspace = useWorkspaceStore();
const boards = useBoardsStore();
const documents = useDocumentsStore();
const folders = useFoldersStore();
const ui = useUiStore();

const activeBoardId = computed(() => {
  const id = route.params.boardId;
  return typeof id === 'string' ? id : null;
});

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const collapsed = ref<Set<string>>(new Set());
function isExpanded(slug: string): boolean {
  return !collapsed.value.has(slug);
}
function toggleProject(slug: string): void {
  const next = new Set(collapsed.value);
  if (next.has(slug)) next.delete(slug);
  else next.add(slug);
  collapsed.value = next;
}

async function loadAll(): Promise<void> {
  const wsSlug = workspace.activeWorkspaceSlug;
  if (wsSlug === null) {
    await workspace.loadProjects('');
    return;
  }

  if (workspace.projects.length === 0) {
    await workspace.loadProjects(wsSlug);
  }

  await Promise.all(workspace.projects.map((p) => boards.loadBoardsForProject(wsSlug, p.slug)));
}

function openBoard(boardId: string): void {
  void router.push({ name: 'tasks', params: { boardId } });
}

// Shared sidebar context-menu + inline-edit logic (same composables as the notes sidebar).
const { open: menuOpen, x: menuX, y: menuY, openAt, close: closeMenu } = useContextMenu();

type EditCtx =
  | { kind: 'new-project' }
  | { kind: 'new-board'; projectSlug: string }
  | { kind: 'rename-board'; boardId: string; projectSlug: string }
  | { kind: 'rename-project'; slug: string }
  | { kind: 'new-doc'; projectSlug: string }
  | { kind: 'new-folder'; projectSlug: string };

const {
  active: editActive,
  value: editValue,
  inputRef,
  start: startEdit,
  commit: commitEdit,
  onKeydown: onEditKeydown,
} = useInlineEdit<EditCtx>(async (name, ctx) => {
  if (ws.value === '') return;

  if (ctx.kind === 'new-project') {
    const slug = await workspace.createProject(ws.value, name);
    if (slug !== null) {
      await boards.loadBoardsForProject(ws.value, slug);
    } else if (workspace.error) {
      ui.showBanner(workspace.error, 'error');
    }
    return;
  }

  if (ctx.kind === 'new-board') {
    const id = await boards.createBoard(ws.value, ctx.projectSlug, name);
    if (id !== null) openBoard(id);
    else if (boards.error) ui.showBanner(boards.error, 'error');
    return;
  }

  if (ctx.kind === 'rename-project') {
    const ok = await workspace.renameProject(ws.value, ctx.slug, name);
    if (!ok && workspace.error) ui.showBanner(workspace.error, 'error');
    return;
  }

  if (ctx.kind === 'new-doc') {
    const slug = await documents.create(ws.value, ctx.projectSlug, name);
    if (slug !== null) void router.push({ name: 'notes', params: { slug } });
    else if (documents.error) ui.showBanner(documents.error, 'error');
    return;
  }

  if (ctx.kind === 'new-folder') {
    const ok = await folders.create(ws.value, ctx.projectSlug, name);
    if (ok) ui.showBanner('Folder created', 'success');
    else if (folders.error) ui.showBanner(folders.error, 'error');
    return;
  }

  const ok = await boards.renameBoard(ws.value, ctx.projectSlug, ctx.boardId, name);
  if (!ok && boards.error) ui.showBanner(boards.error, 'error');
});

async function removeBoard(projectSlug: string, boardId: string): Promise<void> {
  if (ws.value === '') return;
  const ok = await boards.removeBoard(ws.value, projectSlug, boardId);
  if (!ok && boards.error) ui.showBanner(boards.error, 'error');
}

const deleteTarget = ref<{ slug: string; name: string } | null>(null);

function boardStillExists(id: string): boolean {
  return workspace.projects.some((p) => boards.boardsFor(p.slug).some((b) => b.id === id));
}

async function confirmDeleteProject(): Promise<void> {
  const target = deleteTarget.value;
  deleteTarget.value = null;
  if (target === null || ws.value === '') return;

  const ok = await workspace.deleteProject(ws.value, target.slug);
  if (!ok) {
    if (workspace.error) ui.showBanner(workspace.error, 'error');
    return;
  }

  await loadAll();
  if (activeBoardId.value !== null && !boardStillExists(activeBoardId.value)) {
    void router.push({ name: 'tasks' });
  }
  ui.showBanner('Project deleted', 'success');
}

type MenuTarget =
  | { kind: 'root' }
  | { kind: 'project'; slug: string }
  | { kind: 'board'; boardId: string; name: string; projectSlug: string };
const menuTarget = ref<MenuTarget>({ kind: 'root' });

const activeMenuItems = computed<MenuItem[]>(() => {
  const t = menuTarget.value;

  if (t.kind === 'board') {
    return [
      { header: true, label: t.name },
      { label: 'Open', icon: 'external-link', kbd: ['↵'], action: () => openBoard(t.boardId) },
      { sep: true },
      {
        label: 'Rename',
        icon: 'pencil',
        kbd: ['F2'],
        action: () =>
          startEdit({ kind: 'rename-board', boardId: t.boardId, projectSlug: t.projectSlug }, t.name, true),
      },
      { sep: true },
      {
        label: 'Delete',
        icon: 'trash',
        kbd: ['⌫'],
        danger: true,
        action: () => removeBoard(t.projectSlug, t.boardId),
      },
    ];
  }

  if (t.kind === 'project') {
    const slug = t.slug;
    const name = workspace.projects.find((p) => p.slug === slug)?.name ?? 'Project';
    return [
      { header: true, label: name },
      {
        label: 'New document',
        icon: 'file-plus',
        action: () => startEdit({ kind: 'new-doc', projectSlug: slug }),
      },
      {
        label: 'New folder',
        icon: 'folder-plus',
        action: () => startEdit({ kind: 'new-folder', projectSlug: slug }),
      },
      {
        label: 'New board',
        icon: 'columns-3',
        action: () => startEdit({ kind: 'new-board', projectSlug: slug }),
      },
      { sep: true },
      {
        label: 'Rename',
        icon: 'pencil',
        kbd: ['F2'],
        action: () => startEdit({ kind: 'rename-project', slug }, name, true),
      },
      {
        label: 'Delete',
        icon: 'trash',
        danger: true,
        action: () => {
          deleteTarget.value = { slug, name };
        },
      },
      { sep: true },
      { label: 'New project', icon: 'folder-plus', action: () => startEdit({ kind: 'new-project' }) },
    ];
  }

  return [{ label: 'New project', icon: 'folder-plus', action: () => startEdit({ kind: 'new-project' }) }];
});

function openRootMenu(event: MouseEvent): void {
  menuTarget.value = { kind: 'root' };
  openAt(event);
}

function openProjectMenu(event: MouseEvent, slug: string): void {
  menuTarget.value = { kind: 'project', slug };
  openAt(event);
}

function openBoardMenu(event: MouseEvent, boardId: string, name: string, projectSlug: string): void {
  menuTarget.value = { kind: 'board', boardId, name, projectSlug };
  openAt(event);
}

defineExpose({ openNewProject: () => startEdit({ kind: 'new-project' }) });

onMounted(loadAll);
watch(() => workspace.activeWorkspaceSlug, loadAll);
</script>

<template>
  <div style="min-height: 100%;" @contextmenu.prevent="openRootMenu">
    <div class="tasks-sidebar-header">
      <SectionLabel>Projects</SectionLabel>
      <button
        type="button"
        class="tasks-sidebar-add"
        title="New project"
        aria-label="New project"
        @click.stop="openRootMenu"
      >
        <Icon name="more-horizontal" :size="14" />
      </button>
    </div>

    <template v-for="p in workspace.projects" :key="p.slug">
      <div
        v-if="editActive?.kind === 'rename-project' && editActive.slug === p.slug"
        style="display: flex; align-items: center; gap: 6px; padding: 3px 8px 3px 8px;"
      >
        <Icon name="folder" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
        <input
          ref="inputRef"
          v-model="editValue"
          type="text"
          placeholder="Project name…"
          class="tasks-inline-input"
          @keydown="onEditKeydown"
          @blur="commitEdit"
        />
      </div>
      <Row
        v-else
        :label="p.name"
        :icon="isExpanded(p.slug) ? 'folder-open' : 'folder'"
        chevron
        :open="isExpanded(p.slug)"
        menu
        @click="toggleProject(p.slug)"
        @menu="(event: MouseEvent) => openProjectMenu(event, p.slug)"
        @contextmenu.prevent.stop="(event: MouseEvent) => openProjectMenu(event, p.slug)"
      />

      <div
        v-if="(editActive?.kind === 'new-doc' || editActive?.kind === 'new-folder') && editActive.projectSlug === p.slug"
        style="display: flex; align-items: center; gap: 6px; padding: 3px 8px 3px 22px;"
      >
        <Icon
          :name="editActive.kind === 'new-doc' ? 'file' : 'folder'"
          :size="13"
          style="color: var(--c-muted); flex-shrink: 0;"
        />
        <input
          ref="inputRef"
          v-model="editValue"
          type="text"
          :placeholder="editActive.kind === 'new-doc' ? 'Page name…' : 'Folder name…'"
          class="tasks-inline-input"
          @keydown="onEditKeydown"
          @blur="commitEdit"
        />
      </div>

      <template v-if="isExpanded(p.slug)">
        <template v-for="b in boards.boardsFor(p.slug)" :key="b.id">
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
            @menu="(event: MouseEvent) => openBoardMenu(event, b.id, b.name, p.slug)"
            @contextmenu.prevent.stop="(event: MouseEvent) => openBoardMenu(event, b.id, b.name, p.slug)"
          />
        </template>

        <div
          v-if="editActive?.kind === 'new-board' && editActive.projectSlug === p.slug"
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
      </template>
    </template>

    <div
      v-if="editActive?.kind === 'new-project'"
      style="display: flex; align-items: center; gap: 6px; padding: 3px 8px 3px 8px;"
    >
      <Icon name="folder" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
      <input
        ref="inputRef"
        v-model="editValue"
        type="text"
        placeholder="Project name…"
        class="tasks-inline-input"
        @keydown="onEditKeydown"
        @blur="commitEdit"
      />
    </div>

    <p
      v-if="workspace.projects.length === 0 && editActive === null"
      style="padding: 8px; font-size: var(--fs-sm); color: var(--c-muted);"
    >
      No projects yet.
    </p>

    <ContextMenu
      :open="menuOpen"
      :x="menuX"
      :y="menuY"
      :items="activeMenuItems"
      @close="closeMenu"
    />

    <ConfirmDialog
      :open="deleteTarget !== null"
      tone="danger"
      title="Delete project?"
      :message="`This permanently deletes “${deleteTarget?.name ?? ''}” and all its boards, folders and documents.`"
      :detail="deleteTarget?.name"
      detail-icon="folder"
      confirm-label="Delete project"
      confirm-icon="trash-2"
      @confirm="confirmDeleteProject"
      @cancel="deleteTarget = null"
    />
  </div>
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
