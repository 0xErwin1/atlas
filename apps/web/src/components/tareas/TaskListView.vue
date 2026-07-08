<script setup lang="ts">
/**
 * Task List view: the board's tasks as vertically grouped rows, mirroring the
 * hi-fi List design (taskboards.jsx). Rows are grouped by `ui.taskGroupBy`
 * (status / assignee / priority); each group has a colored header with a count
 * and lists its tasks. A row shows a leading status marker, title, label chips,
 * priority, estimate, assignee, and the mono readable id. Clicking a row emits
 * `select` so the host opens the detail pane; the selected row is highlighted.
 *
 * All data is real (read from the boards store). The status color is the user's
 * registry choice for the column value, never inferred from text.
 */
import { computed, ref } from 'vue';
import { VueDraggable } from 'vue-draggable-plus';
import TaskListRow from '@/components/tareas/TaskListRow.vue';
import type { PickerOption } from '@/components/tareas/TaskRowPicker.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import { resolveDropTarget } from '@/composables/kanbanDrop';
import { useContextMenu } from '@/composables/useContextMenu';
import {
  type DragAutoScrollMoveEvent,
  dragAutoScrollOptions,
  handleDragAutoScrollMove,
} from '@/composables/useDragAutoScroll';
import { useInlineEdit } from '@/composables/useInlineEdit';
import { useKanbanMove } from '@/composables/useKanbanMove';
import { useTaskInteractions } from '@/composables/useTaskInteractions';
import { resolveColumnSwatchId } from '@/lib/columnColor';
import { swatchById } from '@/lib/swatches';
import { PRIORITY_COLOR, priorityLabel } from '@/lib/taskPriority';
import type { ColumnDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';
import { type TaskViewMode, useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const props = defineProps<{ ws: string; selectedReadableId: string | null }>();
const emit = defineEmits<{
  select: [readableId: string, mode?: TaskViewMode];
  open: [readableId: string];
}>();

const boards = useBoardsStore();
const workspace = useWorkspaceStore();
const ui = useUiStore();
const menu = useContextMenu();
const ti = useTaskInteractions(props.ws);
const { move } = useKanbanMove(props.ws);

/**
 * Drag-drop is valid only when grouped by status: the group key is then a real
 * column_id that the move API accepts. For assignee/priority grouping the key is
 * not a column_id, so we suppress the draggable wrapper entirely in those modes.
 */
const isDragEnabled = computed(() => ui.taskGroupBy === 'status');

// Inline create, only offered under status grouping where a group's key IS a
// real column_id; the ctx threaded through the input is that target column.
const {
  active: adding,
  value: addValue,
  inputRef,
  start: startAdd,
  commit: commitAdd,
  cancel: cancelAdd,
  onKeydown: onAddKeydown,
} = useInlineEdit<string>((title, columnId) => void createTaskInColumn(columnId, title));

async function createTaskInColumn(columnId: string, title: string): Promise<void> {
  const boardId = boards.board?.id;
  if (boardId === undefined) return;

  const created = await boards.createTask(props.ws, boardId, columnId, title);
  if (created !== null) emit('select', created);
  else if (boards.error !== null) ui.showBanner(boards.error, 'error');
}

async function onSortableDrop(event: unknown, columnId: string): Promise<void> {
  const target = resolveDropTarget(event as Parameters<typeof resolveDropTarget>[0]);
  if (target === null) return;

  const result = await move(target.readableId, columnId, target.toIndex);
  if (!result.ok) {
    ui.showBanner(result.hint ?? 'Move failed', 'error');
  }
}

function onSortableMove(event: DragAutoScrollMoveEvent, originalEvent: Event): void {
  handleDragAutoScrollMove(event, originalEvent, listScrollRef.value);
}

interface Group {
  key: string;
  label: string;
  /** Swatch fg color for the header dot; null falls back to the muted token. */
  color: string | null;
  /** Marks rows in a terminal "done" column so the leading marker fills. */
  done: boolean;
  tasks: TaskSummaryDto[];
}

const PRIORITY_ORDER = ['urgent', 'high', 'medium', 'low'] as const;

function statusColor(column: ColumnDto): string {
  return swatchById(resolveColumnSwatchId(column)).fg;
}

function isDoneColumn(column: ColumnDto): boolean {
  return column.name.trim().toLowerCase() === 'done';
}

const allTasks = computed<TaskSummaryDto[]>(() =>
  boards.columns.flatMap((c) => boards.filteredTasksByColumn(c.id)),
);

function firstAssigneeName(task: TaskSummaryDto): string | null {
  const actor = task.assignees?.[0];
  if (actor === undefined) return null;
  return actor.display_name ?? (actor.type === 'api_key' ? 'Agent' : 'User');
}

const statusGroups = computed<Group[]>(() =>
  boards.columns.map((column) => ({
    key: column.id,
    label: column.name,
    color: statusColor(column),
    done: isDoneColumn(column),
    tasks: boards.filteredTasksByColumn(column.id),
  })),
);

const assigneeGroups = computed<Group[]>(() => {
  const buckets = new Map<string, TaskSummaryDto[]>();

  for (const task of allTasks.value) {
    const name = firstAssigneeName(task) ?? 'Unassigned';
    const bucket = buckets.get(name) ?? [];
    bucket.push(task);
    buckets.set(name, bucket);
  }

  const names = [...buckets.keys()].sort((a, b) => {
    if (a === 'Unassigned') return 1;
    if (b === 'Unassigned') return -1;
    return a.localeCompare(b);
  });

  return names.map((name) => ({
    key: name,
    label: name,
    color: null,
    done: false,
    tasks: buckets.get(name) ?? [],
  }));
});

const priorityGroups = computed<Group[]>(() => {
  const buckets = new Map<string, TaskSummaryDto[]>();

  for (const task of allTasks.value) {
    const key = task.priority ?? '';
    const bucket = buckets.get(key) ?? [];
    bucket.push(task);
    buckets.set(key, bucket);
  }

  const ordered: string[] = [...PRIORITY_ORDER].filter((p) => buckets.has(p));
  if (buckets.has('')) ordered.push('');

  return ordered.map((key) => ({
    key: key === '' ? 'none' : key,
    label: priorityLabel(key === '' ? null : key),
    color: key === '' ? null : (PRIORITY_COLOR[key] ?? null),
    done: false,
    tasks: buckets.get(key) ?? [],
  }));
});

const groups = computed<Group[]>(() => {
  if (ui.taskGroupBy === 'assignee') return assigneeGroups.value;
  if (ui.taskGroupBy === 'priority') return priorityGroups.value;
  return statusGroups.value;
});

// A task's column, looked up by id once per render instead of a linear
// `columns.find` on each of the several per-row status helpers (taskIsDone alone
// runs twice per row).
const columnById = computed<Map<string, ColumnDto>>(() => {
  const map = new Map<string, ColumnDto>();
  for (const column of boards.columns) map.set(column.id, column);
  return map;
});

function statusRingColor(task: TaskSummaryDto): string {
  const column = columnById.value.get(task.column_id);
  return column !== undefined ? statusColor(column) : 'var(--c-muted)';
}

function taskIsDone(task: TaskSummaryDto): boolean {
  const column = columnById.value.get(task.column_id);
  return column !== undefined && isDoneColumn(column);
}

function statusNameForTask(task: TaskSummaryDto): string {
  const column = columnById.value.get(task.column_id);
  return column?.name ?? '';
}

// Session-only collapse state per group; v-show (not v-if) keeps the rows mounted
// so later drag-drop zones survive a collapse toggle.
const collapsedGroups = ref<Set<string>>(new Set());
const listScrollRef = ref<HTMLElement | null>(null);

function isGroupCollapsed(key: string): boolean {
  return collapsedGroups.value.has(key);
}

function toggleGroup(key: string): void {
  const next = new Set(collapsedGroups.value);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  collapsedGroups.value = next;
}

async function copyId(task: TaskSummaryDto): Promise<void> {
  await ti.copyText(task.readable_id, task.readable_id);
}

async function onMenu(task: TaskSummaryDto, event: MouseEvent): Promise<void> {
  ti.menuReadableId.value = task.readable_id;
  menu.openAt(event);

  void workspace.loadMembers(props.ws);
  await Promise.all(workspace.projects.map((p) => boards.loadBoardsForProject(props.ws, p.slug)));
}

const menuItems = computed(() => {
  const readableId = ti.menuReadableId.value;
  if (readableId === null) return [];

  const task = findRowTask(readableId);
  if (task === undefined) return [];

  const boardId = boards.board?.id;
  return ti.buildMenuItems({
    task,
    boardId,
    columns: boards.columns,
    allowDuplicate: boardId !== undefined,
    onOpen: (rid) => emit('open', rid),
    onOpenAs: (rid, mode) => emit('select', rid, mode),
  });
});

const deleteTarget = computed(() => ti.deleteTargetFor(ti.menuReadableId.value));

const PRIORITY_OPTIONS = [
  { value: 'urgent', label: 'Urgent' },
  { value: 'high', label: 'High' },
  { value: 'medium', label: 'Medium' },
  { value: 'low', label: 'Low' },
] as const;

// One open picker at a time, keyed by `${kind}:${task.id}`; closed on list scroll
// because the teleported (fixed) panel does not follow the trigger.
const openPickerKey = ref<string | null>(null);

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
  return boards.columns.map((column) => ({
    value: column.id,
    label: column.name,
    color: statusColor(column),
    active: column.id === task.column_id,
  }));
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

function assignedRefs(task: TaskSummaryDto): Set<string> {
  return new Set((task.assignees ?? []).map((a) => `${a.type}:${a.id}`));
}

function assigneeOptionsFor(task: TaskSummaryDto): PickerOption[] {
  const assigned = assignedRefs(task);
  return workspace.members.map((member) => {
    const value = `${member.principal_type}:${member.id}`;
    return {
      value,
      label: member.display,
      icon: member.principal_type === 'api_key' ? 'bot' : 'user',
      active: assigned.has(value),
    };
  });
}

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

  // Toggle: picking an already-assigned principal removes it instead of
  // re-posting an assignment the server would reject as a 409 conflict.
  if (assignedRefs(task).has(value)) {
    void ti.runUnassign(task.readable_id, type, id);
    return;
  }

  void ti.runAssign(task.readable_id, type, id);
}

// The board list excludes child tasks, so a right-clicked sub-task is resolved
// from the expansion cache instead of the board columns.
function findRowTask(readableId: string): TaskSummaryDto | undefined {
  const top = boards.findTaskByReadableId(readableId);
  if (top !== undefined) return top;

  for (const children of subtaskCache.value.values()) {
    const found = children.find((c) => c.readable_id === readableId);
    if (found !== undefined) return found;
  }
  return undefined;
}

// Sub-task expansion (tree): which rows are open, plus a per-parent cache of the
// fetched children so re-expanding is instant. Keyed by readable_id.
const expandedTasks = ref<Set<string>>(new Set());
const subtaskCache = ref<Map<string, TaskSummaryDto[]>>(new Map());
const loadingSubtasks = new Set<string>();

function hasSubtasks(task: TaskSummaryDto): boolean {
  return task.subtask_count > 0;
}

function isExpanded(task: TaskSummaryDto): boolean {
  return expandedTasks.value.has(task.readable_id);
}

function childrenOf(task: TaskSummaryDto): TaskSummaryDto[] {
  return subtaskCache.value.get(task.readable_id) ?? [];
}

async function toggleExpand(task: TaskSummaryDto): Promise<void> {
  const id = task.readable_id;
  const next = new Set(expandedTasks.value);

  if (next.has(id)) {
    next.delete(id);
    expandedTasks.value = next;
    return;
  }

  next.add(id);
  expandedTasks.value = next;

  // Fetch once; a cached branch re-expands instantly, and a concurrent toggle is
  // guarded so a double-click never fires two requests.
  if (subtaskCache.value.has(id) || loadingSubtasks.has(id)) return;

  loadingSubtasks.add(id);
  const children = await boards.loadSubtasks(props.ws, id);
  loadingSubtasks.delete(id);

  subtaskCache.value = new Map(subtaskCache.value).set(id, children);
}

// The presentational row takes every derived value and option list as props; this
// bundles them so a board task and each of its nested children bind identically.
function rowProps(task: TaskSummaryDto) {
  return {
    task,
    selected: task.readable_id === props.selectedReadableId,
    done: taskIsDone(task),
    ringColor: statusRingColor(task),
    statusName: statusNameForTask(task),
    statusOptions: statusOptionsFor(task),
    assigneeOptions: assigneeOptionsFor(task),
    priorityOptions: priorityOptionsFor(task),
    statusOpen: isPickerOpen('status', task),
    assigneeOpen: isPickerOpen('assignee', task),
    priorityOpen: isPickerOpen('priority', task),
  };
}

// One handler map shared by every row (parent and child) so the wiring is not
// repeated across the draggable and plain render paths.
const rowHandlers = {
  select: (readableId: string) => emit('select', readableId),
  menu: onMenu,
  copy: copyId,
  toggleExpand,
  statusOpen: (task: TaskSummaryDto, value: boolean) => setPickerOpen('status', task, value),
  assigneeOpen: onAssigneeOpen,
  priorityOpen: (task: TaskSummaryDto, value: boolean) => setPickerOpen('priority', task, value),
  statusPick: onStatusPick,
  assigneePick: onAssigneePick,
  priorityPick: onPriorityPick,
};
</script>

<template>
  <div ref="listScrollRef" class="atl-tl-scroll" @scroll="closePickers">
    <div class="atl-tl-inner">
      <div v-if="groups.length > 0" class="atl-tl-colhead">
        <span />
        <span class="atl-tl-h">Name</span>
        <span class="atl-tl-h atl-tl-h-id">ID</span>
        <span class="atl-tl-h atl-tl-h-center">Assignee</span>
        <span class="atl-tl-h">Priority</span>
        <span class="atl-tl-h atl-tl-h-center">Status</span>
        <span class="atl-tl-h atl-tl-h-id">Estimate</span>
      </div>

      <div v-for="group in groups" :key="group.key" class="atl-tl-group">
        <button
          type="button"
          class="atl-tl-grouphead"
          :aria-expanded="!isGroupCollapsed(group.key)"
          @click="toggleGroup(group.key)"
        >
          <Icon
            name="chevron-down"
            :size="13"
            class="atl-tl-chevron"
            :class="{ collapsed: isGroupCollapsed(group.key) }"
            style="color: var(--c-muted);"
          />
          <span
            class="atl-tl-dot"
            :style="{ background: group.color ?? 'var(--c-muted)' }"
          />
          <span class="atl-tl-groupname">{{ group.label }}</span>
          <span class="atl-tl-count">{{ group.tasks.length }}</span>
        </button>

        <VueDraggable
          v-if="isDragEnabled"
          :model-value="group.tasks"
          v-show="!isGroupCollapsed(group.key)"
          :group="'kanban'"
          :animation="150"
          v-bind="dragAutoScrollOptions"
          :on-move="onSortableMove"
          item-key="id"
          ghost-class="atl-tl-row-ghost"
          @update:model-value="() => undefined"
          @add="(e: unknown) => onSortableDrop(e, group.key)"
          @update="(e: unknown) => onSortableDrop(e, group.key)"
        >
          <div
            v-for="task in group.tasks"
            :key="task.id"
            class="atl-tl-item"
            :data-readable-id="task.readable_id"
          >
            <TaskListRow
              v-bind="rowProps(task)"
              v-on="rowHandlers"
              :expandable="hasSubtasks(task)"
              :expanded="isExpanded(task)"
            />
            <div v-if="isExpanded(task)" class="atl-tl-children">
              <TaskListRow
                v-for="child in childrenOf(task)"
                :key="child.id"
                v-bind="rowProps(child)"
                v-on="rowHandlers"
                :indent="1"
              />
            </div>
          </div>
        </VueDraggable>

        <div v-else v-show="!isGroupCollapsed(group.key)">
          <div v-for="task in group.tasks" :key="task.id" class="atl-tl-item">
            <TaskListRow
              v-bind="rowProps(task)"
              v-on="rowHandlers"
              :expandable="hasSubtasks(task)"
              :expanded="isExpanded(task)"
            />
            <div v-if="isExpanded(task)" class="atl-tl-children">
              <TaskListRow
                v-for="child in childrenOf(task)"
                :key="child.id"
                v-bind="rowProps(child)"
                v-on="rowHandlers"
                :indent="1"
              />
            </div>
          </div>
        </div>

        <div
          v-if="ui.taskGroupBy === 'status'"
          v-show="!isGroupCollapsed(group.key)"
          class="atl-tl-add-row"
        >
          <template v-if="adding === group.key">
            <input
              ref="inputRef"
              v-model="addValue"
              type="text"
              placeholder="Task title…"
              class="atl-tl-add-input"
              aria-label="New task title"
              @keydown="onAddKeydown"
              @blur="commitAdd"
            />
            <button
              type="button"
              class="atl-tl-add-cancel"
              title="Cancel (Esc)"
              aria-label="Cancel task creation"
              @mousedown.prevent="cancelAdd"
            >
              <Icon name="x" :size="13" />
            </button>
          </template>
          <button
            v-else
            type="button"
            class="atl-tl-add"
            @click="startAdd(group.key)"
          >
            <Icon name="plus" :size="13" />
            <span>Add task</span>
          </button>
        </div>
      </div>

      <p v-if="groups.length === 0" class="atl-tl-empty">No tasks to show.</p>
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
  grid-template-columns: 15px minmax(0, 1fr) 84px 64px 96px 110px 64px;
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

.atl-tl-group {
  margin-bottom: 10px;
}

.atl-tl-grouphead {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  height: 30px;
  padding: 0 8px;
  border: none;
  background: transparent;
  text-align: left;
  cursor: pointer;
}

.atl-tl-chevron {
  transition: transform 0.12s ease;
}

.atl-tl-chevron.collapsed {
  transform: rotate(-90deg);
}

.atl-tl-dot {
  width: 7px;
  height: 7px;
  border-radius: var(--r-full);
  flex: 0 0 auto;
}

.atl-tl-groupname {
  font-size: var(--fs-sm);
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-tl-count {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

/* Row cell styles live in TaskListRow.vue; this view owns only the group layout
   and the sub-task tree wrapper. */
.atl-tl-children {
  /* Nested rows indent their name column via the row's `indent` prop; a faint
     rail ties them to the parent above. */
  margin-left: 17px;
  border-left: 1px solid var(--c-border);
}

.atl-tl-add-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 12px 2px 10px;
}

.atl-tl-add {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  height: 32px;
  padding: 0 5px;
  border: none;
  border-radius: 3px;
  background: transparent;
  text-align: left;
  color: var(--c-muted);
  font-size: var(--fs-sm);
  cursor: pointer;
}

.atl-tl-add:hover {
  background: var(--c-raised);
  color: var(--c-foreground);
}

.atl-tl-add-input {
  flex: 1 1 auto;
  min-width: 0;
  height: 32px;
  padding: 0 8px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  outline: none;
  color: var(--c-foreground);
  font-family: var(--font-ui);
  font-size: var(--fs-base);
}

.atl-tl-add-input:focus {
  border-color: var(--c-primary);
}

.atl-tl-add-cancel {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 auto;
  width: 30px;
  height: 30px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
}

.atl-tl-add-cancel:hover {
  background: var(--c-raised);
  color: var(--c-foreground);
}

.atl-tl-empty {
  padding: 8px;
  font-size: var(--fs-sm);
  color: var(--c-muted);
}

.atl-tl-row-ghost {
  opacity: 0.4;
}
</style>
