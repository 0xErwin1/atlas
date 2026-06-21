<script setup lang="ts">
/**
 * Flat task list for cross-board views (predefined and custom). Unlike
 * TaskListView, there is no column grouping and no dependency on the boards
 * store: the status text and board name travel on each TaskSummaryDto
 * (`column_name` / `board_name`), so a single feed can mix tasks from any
 * board. The row visual language is shared verbatim with TaskListView via the
 * `atl-tl-*` classes so the two views feel identical.
 *
 * Clicking a row emits `select` (host opens the detail pane); double-clicking
 * emits `open`. The status color is the user's registry choice for the column
 * value, never inferred from text.
 */
import { computed, ref } from 'vue';
import Avatar from '@/components/ui/Avatar.vue';
import Chip from '@/components/ui/Chip.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { useTaskInteractions } from '@/composables/useTaskInteractions';
import { relativeTime } from '@/lib/relativeTime';
import { swatchById } from '@/lib/swatches';
import type { ColumnDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';
import { useLabelColorsStore } from '@/stores/labelColors';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const props = defineProps<{
  ws: string;
  tasks: TaskSummaryDto[];
  selectedReadableId?: string | null;
}>();

const emit = defineEmits<{
  select: [readableId: string];
  open: [readableId: string];
}>();

const boards = useBoardsStore();
const workspace = useWorkspaceStore();
const labelColors = useLabelColorsStore();
const ui = useUiStore();
const menu = useContextMenu();
const ti = useTaskInteractions(props.ws);

const PRIORITY_COLOR: Record<string, string> = {
  urgent: 'var(--c-danger)',
  high: 'var(--c-primary)',
  medium: 'var(--c-info)',
  low: 'var(--c-muted)',
};

function statusColor(columnName: string): string {
  return swatchById(labelColors.colorFor(`status:${columnName}`)).fg;
}

function isDone(task: TaskSummaryDto): boolean {
  return task.column_name.trim().toLowerCase() === 'done';
}

function priorityLabel(priority: string | null | undefined): string {
  if (priority === null || priority === undefined || priority === '') return 'No priority';
  return priority.charAt(0).toUpperCase() + priority.slice(1);
}

function assigneeForTask(task: TaskSummaryDto): { name: string; agent: boolean } | null {
  const actor = task.assignees?.[0];
  if (actor === undefined) return null;
  const agent = actor.type === 'api_key';
  return { name: actor.display_name ?? (agent ? 'Agent' : 'User'), agent };
}

const isEmpty = computed(() => props.tasks.length === 0);

const menuTaskColumns = ref<ColumnDto[]>([]);

async function copyId(task: TaskSummaryDto): Promise<void> {
  await ti.copyText(task.readable_id, task.readable_id);
}

async function onMenu(task: TaskSummaryDto, event: MouseEvent): Promise<void> {
  ti.menuReadableId.value = task.readable_id;
  menu.openAt(event);

  void workspace.loadMembers(props.ws);
  await Promise.all(workspace.projects.map((p) => boards.loadBoardsForProject(props.ws, p.slug)));

  if (task.board_id !== '') {
    menuTaskColumns.value = await boards.fetchColumnsForBoard(props.ws, task.board_id);
  }
}

const menuItems = computed(() => {
  const readableId = ti.menuReadableId.value;
  if (readableId === null) return [];

  const task = props.tasks.find((t) => t.readable_id === readableId);
  if (task === undefined) return [];

  const boardId = task.board_id !== '' ? task.board_id : undefined;

  return ti.buildMenuItems({
    task,
    boardId,
    columns: menuTaskColumns.value,
    allowDuplicate: boardId !== undefined,
    onOpen: (rid) => emit('open', rid),
  });
});

const deleteTarget = computed(() => {
  const rid = ti.menuReadableId.value;
  if (rid === null) return null;
  return props.tasks.find((t) => t.readable_id === rid) ?? null;
});
</script>

<template>
  <div class="atl-tl-scroll">
    <div class="atl-tl-inner">
      <div v-if="!isEmpty" class="atl-tl-colhead">
        <span />
        <span>Name</span>
        <span>Board</span>
        <span class="atl-tl-h-id">ID</span>
        <span class="atl-tl-h-center">Assignee</span>
        <span>Priority</span>
        <span>Status</span>
        <span class="atl-tl-h-id">Estimate</span>
        <span class="atl-tl-h-id">Updated</span>
      </div>

      <button
        v-for="task in tasks"
        :key="task.id"
        type="button"
        class="atl-tl-row"
        :class="{ selected: task.readable_id === selectedReadableId }"
        @click="emit('select', task.readable_id)"
        @dblclick="emit('open', task.readable_id)"
        @contextmenu.prevent="onMenu(task, $event)"
      >
        <span
          v-if="isDone(task)"
          class="atl-tl-marker done"
        >
          <Icon name="check" :size="10" :stroke-width="2.6" />
        </span>
        <span
          v-else
          class="atl-tl-marker"
          :style="{ borderColor: statusColor(task.column_name) }"
        />

        <span class="atl-tl-name">
          <span class="atl-tl-title" :class="{ muted: isDone(task) }">{{ task.title }}</span>
          <span v-if="(task.labels ?? []).length > 0" class="atl-tl-labels">
            <Chip
              v-for="label in task.labels ?? []"
              :key="label"
              :color="labelColors.colorFor(`tag:${label.toLowerCase()}`)"
            >
              {{ label }}
            </Chip>
          </span>
        </span>

        <span class="atl-tl-board">{{ task.board_name }}</span>

        <span class="atl-tl-id">
          <span class="atl-tl-id-text">{{ task.readable_id }}</span>
          <button
            type="button"
            class="atl-tl-copy"
            :title="`Copy ${task.readable_id}`"
            @click.stop="copyId(task)"
          >
            <Icon name="copy" :size="12" />
          </button>
        </span>

        <span class="atl-tl-assignee">
          <template v-if="assigneeForTask(task)">
            <Avatar :name="assigneeForTask(task)!.name" :agent="assigneeForTask(task)!.agent" :size="18" />
          </template>
          <span v-else class="atl-tl-noassignee" title="Unassigned">
            <Icon name="user" :size="11" />
          </span>
        </span>

        <span class="atl-tl-prio">
          <template v-if="task.priority">
            <Icon
              name="flag"
              :size="13"
              :style="{ color: PRIORITY_COLOR[task.priority] ?? 'var(--c-muted)' }"
            />
            {{ priorityLabel(task.priority) }}
          </template>
        </span>

        <span
          class="atl-tl-status"
          :style="{ color: statusColor(task.column_name) }"
        >
          {{ task.column_name }}
        </span>

        <span class="atl-tl-est">{{ task.estimate !== null && task.estimate !== undefined ? `${task.estimate} pts` : '—' }}</span>

        <span class="atl-tl-updated">{{ relativeTime(task.updated_at) }}</span>
      </button>

      <p v-if="isEmpty" class="atl-tl-empty">No tasks to show.</p>
    </div>
  </div>

  <ContextMenu
    :open="menu.open.value"
    :x="menu.x.value"
    :y="menu.y.value"
    :items="menuItems"
    @close="menu.close"
  />

  <PromptDialog
    :open="ti.promptState.value.open"
    :title="ti.promptState.value.title"
    :initial="ti.promptState.value.initial"
    :input-type="ti.promptState.value.mode === 'due' ? 'date' : 'text'"
    @confirm="ti.onPromptConfirm"
    @cancel="ti.promptState.value = { ...ti.promptState.value, open: false }"
  />

  <ConfirmDialog
    :open="ti.confirmOpen.value"
    tone="danger"
    title="Delete this task?"
    message="The task is removed permanently. This can't be undone."
    :detail="deleteTarget ? `${deleteTarget.readable_id} · ${deleteTarget.title}` : undefined"
    detail-icon="square-kanban"
    note="Its sub-tasks, references, and activity are removed along with it."
    confirm-label="Delete task"
    confirm-icon="trash-2"
    @confirm="ti.onConfirmDelete"
    @cancel="ti.confirmOpen.value = false"
  />
</template>

<style scoped>
.atl-tl-scroll {
  flex: 1 1 0;
  min-width: 0;
  min-height: 0;
  overflow: auto;
  padding: 14px 16px;
  background: var(--c-background);
}

.atl-tl-inner {
  max-width: 1100px;
}

.atl-tl-colhead {
  display: grid;
  grid-template-columns: 15px minmax(0, 1fr) 110px 64px 26px 92px 110px 56px 64px;
  align-items: center;
  column-gap: 10px;
  padding: 0 12px 0 10px;
  height: 28px;
  font-size: var(--fs-xs);
  font-weight: var(--fw-semibold);
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--c-muted);
  border-bottom: 1px solid var(--c-border);
  margin-bottom: 4px;
}

.atl-tl-h-id {
  text-align: right;
}

.atl-tl-h-center {
  text-align: center;
}

.atl-tl-row {
  display: grid;
  grid-template-columns: 15px minmax(0, 1fr) 110px 64px 26px 92px 110px 56px 64px;
  align-items: center;
  column-gap: 10px;
  width: 100%;
  height: 38px;
  padding: 0 12px 0 10px;
  border: none;
  border-radius: 3px;
  background: transparent;
  text-align: left;
  cursor: pointer;
}

.atl-tl-row:hover {
  background: var(--c-raised);
}

.atl-tl-row.selected {
  background: var(--c-selection);
  box-shadow: inset 2px 0 0 var(--c-primary);
}

.atl-tl-marker {
  width: 15px;
  height: 15px;
  border-radius: var(--r-full);
  border: 1.6px solid var(--c-muted);
  flex: 0 0 auto;
}

.atl-tl-marker.done {
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: var(--c-success);
  color: var(--c-background);
}

.atl-tl-status {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-sm);
  font-weight: var(--fw-semibold);
}

.atl-tl-board {
  justify-self: start;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 2px 7px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  background: var(--c-raised);
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-tl-name {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.atl-tl-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-lg);
  color: var(--c-foreground);
}

.atl-tl-title.muted {
  color: var(--c-muted);
}

.atl-tl-labels {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  overflow: hidden;
  flex: 0 1 auto;
}

.atl-tl-prio {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  font-size: var(--fs-sm);
  color: var(--c-foreground);
}

.atl-tl-est {
  text-align: right;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-tl-assignee {
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.atl-tl-noassignee {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border: 1px dashed var(--c-muted);
  border-radius: 2px;
  color: var(--c-muted);
  flex: 0 0 auto;
}

.atl-tl-id {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: 4px;
  min-width: 0;
}

.atl-tl-id-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-tl-copy {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 auto;
  width: 18px;
  height: 18px;
  padding: 0;
  border: none;
  border-radius: 3px;
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.12s ease;
}

.atl-tl-copy:hover {
  background: var(--c-raised);
  color: var(--c-foreground);
}

.atl-tl-row:hover .atl-tl-copy {
  opacity: 1;
}

.atl-tl-updated {
  text-align: right;
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-tl-empty {
  padding: 8px;
  font-size: var(--fs-sm);
  color: var(--c-muted);
}
</style>
