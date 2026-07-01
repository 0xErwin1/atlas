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
import TaskRowPicker, { type PickerOption } from '@/components/tareas/TaskRowPicker.vue';
import Avatar from '@/components/ui/Avatar.vue';
import Chip from '@/components/ui/Chip.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { useTaskInteractions } from '@/composables/useTaskInteractions';
import { resolveColumnSwatchId } from '@/lib/columnColor';
import { relativeTime } from '@/lib/relativeTime';
import { defaultSwatchId, swatchById } from '@/lib/swatches';
import type { ColumnDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';
import { useLabelColorsStore } from '@/stores/labelColors';
import { type TaskViewMode, useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const props = defineProps<{
  ws: string;
  tasks: TaskSummaryDto[];
  selectedReadableId?: string | null;
}>();

const emit = defineEmits<{
  select: [readableId: string, mode?: TaskViewMode];
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

// This cross-board feed carries only the column NAME on each task, not the
// column id/color. When the board's columns have been loaded (e.g. after opening
// a status picker) the backend color is the source of truth via the resolver;
// otherwise the deterministic name-keyed default keeps the marker stable.
function statusColor(columnName: string, boardId?: string): string {
  if (boardId !== undefined) {
    const column = (statusColumns.value[boardId] ?? []).find((c) => c.name === columnName);
    if (column !== undefined) return swatchById(resolveColumnSwatchId(column)).fg;
  }
  return swatchById(defaultSwatchId(`status:${columnName}`)).fg;
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
    onOpenAs: (rid, mode) => emit('select', rid, mode),
  });
});

const deleteTarget = computed(() => {
  const rid = ti.menuReadableId.value;
  if (rid === null) return null;
  return props.tasks.find((t) => t.readable_id === rid) ?? null;
});

const PRIORITY_OPTIONS = [
  { value: 'urgent', label: 'Urgent' },
  { value: 'high', label: 'High' },
  { value: 'medium', label: 'Medium' },
  { value: 'low', label: 'Low' },
] as const;

// One open picker at a time, keyed by `${kind}:${task.id}`; closed on list scroll
// because the teleported (fixed) panel does not follow the trigger.
const openPickerKey = ref<string | null>(null);

// Per-board column cache for the cross-board status picker (a task's options are
// its OWN board's columns, loaded on demand without touching boards.columns).
const statusColumns = ref<Record<string, ColumnDto[]>>({});

function pickerKey(kind: string, task: TaskSummaryDto): string {
  return `${kind}:${task.id}`;
}

function isPickerOpen(kind: string, task: TaskSummaryDto): boolean {
  return openPickerKey.value === pickerKey(kind, task);
}

function setPickerOpen(kind: string, task: TaskSummaryDto, value: boolean): void {
  openPickerKey.value = value ? pickerKey(kind, task) : null;
}

function closePickers(): void {
  openPickerKey.value = null;
}

function statusOptionsFor(task: TaskSummaryDto): PickerOption[] {
  return (statusColumns.value[task.board_id] ?? []).map((column) => ({
    value: column.id,
    label: column.name,
    color: swatchById(resolveColumnSwatchId(column)).fg,
    active: column.id === task.column_id,
  }));
}

async function onStatusOpen(task: TaskSummaryDto, value: boolean): Promise<void> {
  setPickerOpen('status', task, value);
  if (value && task.board_id !== '' && statusColumns.value[task.board_id] === undefined) {
    const columns = await boards.fetchColumnsForBoard(props.ws, task.board_id);
    statusColumns.value = { ...statusColumns.value, [task.board_id]: columns };
  }
}

function priorityOptionsFor(task: TaskSummaryDto): PickerOption[] {
  const options: PickerOption[] = PRIORITY_OPTIONS.map((p) => ({
    value: p.value,
    label: p.label,
    icon: 'flag',
    color: PRIORITY_COLOR[p.value],
    active: task.priority === p.value,
  }));

  options.push({ value: '', label: 'Clear', icon: 'x', muted: true });
  return options;
}

const assigneeOptions = computed<PickerOption[]>(() =>
  workspace.members.map((member) => ({
    value: `${member.principal_type}:${member.id}`,
    label: member.display,
    icon: member.principal_type === 'api_key' ? 'bot' : 'user',
  })),
);

async function onAssigneeOpen(task: TaskSummaryDto, value: boolean): Promise<void> {
  setPickerOpen('assignee', task, value);
  if (value && workspace.members.length === 0) await workspace.loadMembers(props.ws);
}

function onStatusPick(task: TaskSummaryDto, columnId: string): void {
  if (columnId === task.column_id) return;
  void ti.runMoveToColumn(task.readable_id, columnId);
}

function onPriorityPick(task: TaskSummaryDto, value: string): void {
  void ti.runUpdate(task.readable_id, { priority: value === '' ? null : value });
}

function onAssigneePick(task: TaskSummaryDto, value: string): void {
  const [type, id] = value.split(/:(.*)/s);
  if (type === undefined || id === undefined) return;
  void ti.runAssign(task.readable_id, type, id);
}
</script>

<template>
  <div class="atl-tl-scroll" @scroll="closePickers">
    <div class="atl-tl-inner">
      <div v-if="!isEmpty" class="atl-tl-colhead">
        <span />
        <span class="atl-tl-h">Name</span>
        <span class="atl-tl-h">Board</span>
        <span class="atl-tl-h atl-tl-h-id">ID</span>
        <span class="atl-tl-h atl-tl-h-center">Assignee</span>
        <span class="atl-tl-h">Priority</span>
        <span class="atl-tl-h">Status</span>
        <span class="atl-tl-h atl-tl-h-id">Estimate</span>
        <span class="atl-tl-h atl-tl-h-id">Updated</span>
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
        <TaskRowPicker
          class="atl-tl-pick"
          :options="statusOptionsFor(task)"
          :open="isPickerOpen('status', task)"
          @update:open="(v: boolean) => onStatusOpen(task, v)"
          @pick="(v: string) => onStatusPick(task, v)"
        >
          <template #trigger>
            <span
              v-if="isDone(task)"
              class="atl-tl-marker done"
              title="Change status"
            >
              <Icon name="check" :size="10" :stroke-width="2.6" />
            </span>
            <span
              v-else
              class="atl-tl-marker"
              title="Change status"
              :style="{ borderColor: statusColor(task.column_name, task.board_id) }"
            />
          </template>
        </TaskRowPicker>

        <span class="atl-tl-name">
          <span class="atl-tl-title" :class="{ muted: isDone(task) }">{{ task.title }}</span>
          <span v-if="(task.labels ?? []).length > 0" class="atl-tl-labels">
            <Chip
              v-for="label in task.labels ?? []"
              :key="label"
              truncate
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

        <TaskRowPicker
          class="atl-tl-pick atl-tl-assignee"
          :options="assigneeOptions"
          width="220px"
          :open="isPickerOpen('assignee', task)"
          @update:open="(v: boolean) => onAssigneeOpen(task, v)"
          @pick="(v: string) => onAssigneePick(task, v)"
        >
          <template #trigger>
            <template v-if="assigneeForTask(task)">
              <Avatar :name="assigneeForTask(task)!.name" :agent="assigneeForTask(task)!.agent" :size="18" />
            </template>
            <span v-else class="atl-tl-noassignee" title="Assign">
              <Icon name="user" :size="11" />
            </span>
          </template>
        </TaskRowPicker>

        <TaskRowPicker
          class="atl-tl-pick atl-tl-prio"
          :options="priorityOptionsFor(task)"
          :open="isPickerOpen('priority', task)"
          @update:open="(v: boolean) => setPickerOpen('priority', task, v)"
          @pick="(v: string) => onPriorityPick(task, v)"
        >
          <template #trigger>
            <span class="atl-tl-prio-inner" title="Change priority">
              <template v-if="task.priority">
                <Icon
                  name="flag"
                  :size="13"
                  :style="{ color: PRIORITY_COLOR[task.priority] ?? 'var(--c-muted)' }"
                />
                {{ priorityLabel(task.priority) }}
              </template>
              <span v-else class="atl-tl-prio-empty">—</span>
            </span>
          </template>
        </TaskRowPicker>

        <span
          class="atl-tl-status"
          :style="{ color: statusColor(task.column_name, task.board_id) }"
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
  width: 100%;
}

.atl-tl-colhead {
  display: grid;
  grid-template-columns: 15px minmax(0, 1fr) 110px 84px 64px 96px 110px 64px 64px;
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

.atl-tl-h {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-tl-h-id {
  text-align: right;
}

.atl-tl-h-center {
  text-align: center;
}

.atl-tl-row {
  display: grid;
  grid-template-columns: 15px minmax(0, 1fr) 110px 84px 64px 96px 110px 64px 64px;
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

.atl-tl-pick {
  display: inline-flex;
  align-items: center;
  min-width: 0;
}

.atl-tl-marker {
  width: 15px;
  height: 15px;
  border-radius: var(--r-full);
  border: 1.6px solid var(--c-muted);
  flex: 0 0 auto;
  transition: transform 0.1s ease;
}

.atl-tl-marker:hover {
  transform: scale(1.12);
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
  min-width: 0;
}

.atl-tl-prio-inner {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  height: 24px;
  padding: 0 6px;
  border-radius: var(--r-sm);
  font-size: var(--fs-sm);
  color: var(--c-foreground);
}

.atl-tl-prio-inner:hover {
  background: var(--c-raised);
}

.atl-tl-prio-empty {
  color: var(--c-muted);
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

.atl-tl-assignee :deep(.atl-rp-trigger) {
  border-radius: var(--r-full);
}

.atl-tl-assignee :deep(.atl-rp-trigger:hover) {
  background: var(--c-raised);
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
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  min-width: 0;
}

.atl-tl-id-text {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--c-muted);
  white-space: nowrap;
}

.atl-tl-copy {
  position: absolute;
  right: -2px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  padding: 0;
  border: none;
  border-radius: 3px;
  background: var(--c-raised);
  color: var(--c-muted);
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.12s ease;
}

.atl-tl-copy:hover {
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
