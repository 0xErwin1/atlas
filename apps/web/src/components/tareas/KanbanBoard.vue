<script setup lang="ts">
import { computed, ref } from 'vue';
import { useRouter } from 'vue-router';
import KanbanColumn from '@/components/tareas/KanbanColumn.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { useKanbanMove } from '@/composables/useKanbanMove';
import { useBoardsStore } from '@/stores/boards';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const props = defineProps<{
  ws: string;
  selectedReadableId?: string | null;
}>();

const emit = defineEmits<{
  select: [readableId: string];
  open: [readableId: string];
}>();

const boards = useBoardsStore();
const workspace = useWorkspaceStore();
const ui = useUiStore();
const router = useRouter();
const { move } = useKanbanMove(props.ws);

const PRIORITIES = ['urgent', 'high', 'medium', 'low'] as const;

const menu = useContextMenu();
const menuReadableId = ref<string | null>(null);

const promptState = ref<{ open: boolean; mode: 'rename' | 'due'; title: string; initial: string }>({
  open: false,
  mode: 'rename',
  title: '',
  initial: '',
});
const confirmOpen = ref(false);

async function onDrop(readableId: string, columnId: string, toIndex: number): Promise<void> {
  const result = await move(readableId, columnId, toIndex);
  if (!result.ok) {
    ui.showBanner(result.hint ?? 'Move failed', 'error');
  }
}

async function onCreate(columnId: string, title: string): Promise<void> {
  const boardId = boards.board?.id;
  if (boardId === undefined) return;

  const created = await boards.createTask(props.ws, boardId, columnId, title);
  if (created === null && boards.error) {
    ui.showBanner(boards.error, 'error');
  }
}

function taskHref(readableId: string): string {
  return router.resolve({ name: 'task-detail', params: { readableId } }).href;
}

async function onMenu(readableId: string, event: MouseEvent): Promise<void> {
  menuReadableId.value = readableId;
  menu.openAt(event);

  // Load the data the submenus need; the items computed reacts as it arrives.
  void workspace.loadMembers(props.ws);
  await Promise.all(workspace.projects.map((p) => boards.loadBoardsForProject(props.ws, p.slug)));
}

async function runUpdate(readableId: string, patch: Parameters<typeof boards.updateTask>[2]): Promise<void> {
  const ok = await boards.updateTask(props.ws, readableId, patch);
  if (!ok && boards.error) ui.showBanner(boards.error, 'error');
}

async function runMoveToColumn(readableId: string, columnId: string): Promise<void> {
  const ok = await boards.moveTaskToColumn(props.ws, readableId, columnId);
  if (!ok && boards.error) ui.showBanner(boards.error, 'error');
}

async function runMoveToBoard(readableId: string, boardId: string): Promise<void> {
  const ok = await boards.moveTaskToBoard(props.ws, readableId, boardId);
  if (ok) ui.showBanner('Task moved to board', 'success');
  else if (boards.error) ui.showBanner(boards.error, 'error');
}

async function runAssign(readableId: string, type: string, id: string): Promise<void> {
  const ok = await boards.assignTask(props.ws, readableId, type, id);
  if (ok) ui.showBanner('Task assigned', 'success');
  else if (boards.error) ui.showBanner(boards.error, 'error');
}

async function runDuplicate(readableId: string): Promise<void> {
  const boardId = boards.board?.id;
  if (boardId === undefined) return;
  const created = await boards.duplicateTask(props.ws, boardId, readableId);
  if (created === null && boards.error) ui.showBanner(boards.error, 'error');
}

async function copyText(text: string, label: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    ui.showBanner(`${label} copied`, 'success');
  } catch {
    ui.showBanner('Clipboard is not available', 'error');
  }
}

function openRename(task: { title: string }): void {
  promptState.value = { open: true, mode: 'rename', title: 'Rename task', initial: task.title };
}

function openDueDate(): void {
  promptState.value = { open: true, mode: 'due', title: 'Set due date', initial: '' };
}

async function onPromptConfirm(value: string): Promise<void> {
  const readableId = menuReadableId.value;
  promptState.value = { ...promptState.value, open: false };
  if (readableId === null) return;

  if (promptState.value.mode === 'rename') {
    const title = value.trim();
    if (title.length > 0) await runUpdate(readableId, { title });
    return;
  }

  // Empty date clears the due date; otherwise send an ISO datetime.
  const due = value === '' ? null : new Date(value).toISOString();
  await runUpdate(readableId, { due_date: due });
}

async function onConfirmDelete(): Promise<void> {
  const readableId = menuReadableId.value;
  confirmOpen.value = false;
  if (readableId === null) return;

  const ok = await boards.deleteTask(props.ws, readableId);
  if (!ok && boards.error) ui.showBanner(boards.error, 'error');
}

const menuItems = computed<MenuItem[]>(() => {
  const readableId = menuReadableId.value;
  if (readableId === null) return [];

  const task = boards.findTaskByReadableId(readableId);
  if (task === undefined) return [];

  const statusChildren: MenuItem[] = boards.columns.map((c) => ({
    label: c.name,
    disabled: c.id === task.column_id,
    action: () => runMoveToColumn(readableId, c.id),
  }));

  const priorityChildren: MenuItem[] = [
    ...PRIORITIES.map((p) => ({
      label: p.charAt(0).toUpperCase() + p.slice(1),
      disabled: task.priority === p,
      action: () => runUpdate(readableId, { priority: p }),
    })),
    { sep: true },
    { label: 'Clear', action: () => runUpdate(readableId, { priority: null }) },
  ];

  const assignChildren: MenuItem[] =
    workspace.members.length > 0
      ? workspace.members.map((m) => ({
          label: m.display,
          icon: m.principal_type === 'api_key' ? 'bot' : 'user',
          action: () => runAssign(readableId, m.principal_type, m.id),
        }))
      : [{ label: 'No members', disabled: true }];

  const activeBoardId = boards.board?.id;
  const otherBoards: MenuItem[] = workspace.projects
    .flatMap((p) => boards.boardsFor(p.slug).map((b) => ({ board: b, projectName: p.name })))
    .filter((entry) => entry.board.id !== activeBoardId)
    .map((entry) => ({
      label: `${entry.projectName} / ${entry.board.name}`,
      action: () => runMoveToBoard(readableId, entry.board.id),
    }));

  const moveChildren: MenuItem[] =
    otherBoards.length > 0 ? otherBoards : [{ label: 'No other boards', disabled: true }];

  return [
    { label: 'Open', icon: 'square-arrow-out-up-right', action: () => emit('open', readableId) },
    {
      label: 'Open in new tab',
      icon: 'external-link',
      action: () => window.open(`${window.location.origin}${taskHref(readableId)}`, '_blank'),
    },
    { sep: true },
    { label: 'Rename', icon: 'pencil', action: () => openRename(task) },
    { label: 'Change status', icon: 'square-kanban', children: statusChildren },
    { label: 'Change priority', icon: 'flag', children: priorityChildren },
    { label: 'Assign to', icon: 'user-plus', children: assignChildren },
    { label: 'Move to board', icon: 'arrow-right-left', children: moveChildren },
    { label: 'Set due date', icon: 'calendar', action: openDueDate },
    { sep: true },
    { label: 'Copy ID', icon: 'hash', action: () => copyText(readableId, 'ID') },
    {
      label: 'Copy link',
      icon: 'link',
      action: () => copyText(`${window.location.origin}${taskHref(readableId)}`, 'Link'),
    },
    { label: 'Duplicate', icon: 'copy', action: () => runDuplicate(readableId) },
    { sep: true },
    { label: 'Delete', icon: 'trash-2', danger: true, action: () => (confirmOpen.value = true) },
  ];
});
</script>

<template>
  <div
    class="flex flex-1 overflow-x-auto"
    style="gap: 14px; padding: 16px; background-color: var(--c-background);"
  >
    <KanbanColumn
      v-for="column in boards.columns"
      :key="column.id"
      :column="column"
      :tasks="boards.tasksByColumn(column.id)"
      :selected-readable-id="selectedReadableId"
      @drop="onDrop"
      @create="onCreate"
      @select="(id) => emit('select', id)"
      @open="(id) => emit('open', id)"
      @menu="onMenu"
    />

    <p
      v-if="boards.columns.length === 0 && !boards.loading"
      style="font-size: var(--fs-sm); color: var(--c-muted); padding: 8px;"
    >
      This board has no columns yet.
    </p>

    <ContextMenu
      :open="menu.open.value"
      :x="menu.x.value"
      :y="menu.y.value"
      :items="menuItems"
      @close="menu.close"
    />

    <PromptDialog
      :open="promptState.open"
      :title="promptState.title"
      :initial="promptState.initial"
      :input-type="promptState.mode === 'due' ? 'date' : 'text'"
      @confirm="onPromptConfirm"
      @cancel="promptState = { ...promptState, open: false }"
    />

    <ConfirmDialog
      :open="confirmOpen"
      title="Delete task"
      message="This task will be permanently removed. This cannot be undone."
      confirm-label="Delete"
      danger
      @confirm="onConfirmDelete"
      @cancel="confirmOpen = false"
    />
  </div>
</template>
