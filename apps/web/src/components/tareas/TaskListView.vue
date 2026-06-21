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
import Avatar from '@/components/ui/Avatar.vue';
import Chip from '@/components/ui/Chip.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { useTaskInteractions } from '@/composables/useTaskInteractions';
import { swatchById } from '@/lib/swatches';
import type { ColumnDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';
import { useLabelColorsStore } from '@/stores/labelColors';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const props = defineProps<{ ws: string; selectedReadableId: string | null }>();
const emit = defineEmits<{ select: [readableId: string]; open: [readableId: string] }>();

const boards = useBoardsStore();
const workspace = useWorkspaceStore();
const labelColors = useLabelColorsStore();
const ui = useUiStore();
const menu = useContextMenu();
const ti = useTaskInteractions(props.ws);

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

const PRIORITY_COLOR: Record<string, string> = {
  urgent: 'var(--c-danger)',
  high: 'var(--c-primary)',
  medium: 'var(--c-info)',
  low: 'var(--c-muted)',
};

function statusColor(column: ColumnDto): string {
  return swatchById(labelColors.colorFor(`status:${column.name}`)).fg;
}

function isDoneColumn(column: ColumnDto): boolean {
  return column.name.trim().toLowerCase() === 'done';
}

function priorityLabel(priority: string | null): string {
  if (priority === null || priority === '') return 'No priority';
  return priority.charAt(0).toUpperCase() + priority.slice(1);
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

function statusRingColor(task: TaskSummaryDto): string {
  const column = boards.columns.find((c) => c.id === task.column_id);
  return column !== undefined ? statusColor(column) : 'var(--c-muted)';
}

function taskIsDone(task: TaskSummaryDto): boolean {
  const column = boards.columns.find((c) => c.id === task.column_id);
  return column !== undefined && isDoneColumn(column);
}

function assigneeForTask(task: TaskSummaryDto): { name: string; agent: boolean } | null {
  const actor = task.assignees?.[0];
  if (actor === undefined) return null;
  const agent = actor.type === 'api_key';
  return { name: actor.display_name ?? (agent ? 'Agent' : 'User'), agent };
}

function statusNameForTask(task: TaskSummaryDto): string {
  const column = boards.columns.find((c) => c.id === task.column_id);
  return column?.name ?? '';
}

// Session-only collapse state per group; v-show (not v-if) keeps the rows mounted
// so later drag-drop zones survive a collapse toggle.
const collapsedGroups = ref<Set<string>>(new Set());

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

  const task = boards.findTaskByReadableId(readableId);
  if (task === undefined) return [];

  const boardId = boards.board?.id;
  return ti.buildMenuItems({
    task,
    boardId,
    columns: boards.columns,
    allowDuplicate: boardId !== undefined,
    onOpen: (rid) => emit('open', rid),
  });
});

const deleteTarget = computed(() => ti.deleteTargetFor(ti.menuReadableId.value));
</script>

<template>
  <div class="atl-tl-scroll">
    <div class="atl-tl-inner">
      <div v-if="groups.length > 0" class="atl-tl-colhead">
        <span />
        <span>Name</span>
        <span class="atl-tl-h-id">ID</span>
        <span class="atl-tl-h-center">Assignee</span>
        <span>Priority</span>
        <span class="atl-tl-h-center">Status</span>
        <span class="atl-tl-h-id">Estimate</span>
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

        <div v-show="!isGroupCollapsed(group.key)">
          <button
            v-for="task in group.tasks"
            :key="task.id"
            type="button"
            class="atl-tl-row"
            :class="{ selected: task.readable_id === selectedReadableId }"
            @click="emit('select', task.readable_id)"
            @contextmenu.prevent="onMenu(task, $event)"
          >
            <span
              v-if="taskIsDone(task)"
              class="atl-tl-marker done"
            >
              <Icon name="check" :size="10" :stroke-width="2.6" />
            </span>
            <span
              v-else
              class="atl-tl-marker"
              :style="{ borderColor: statusRingColor(task) }"
            />

            <span class="atl-tl-name">
              <span class="atl-tl-title" :class="{ muted: taskIsDone(task) }">{{ task.title }}</span>
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

            <span class="atl-tl-status">{{ statusNameForTask(task) }}</span>

            <span class="atl-tl-est">{{ task.estimate !== null && task.estimate !== undefined ? `${task.estimate} pts` : '—' }}</span>
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
  max-width: 1100px;
}

.atl-tl-colhead {
  display: grid;
  grid-template-columns: 15px minmax(0, 1fr) 64px 26px 92px 110px 56px;
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

.atl-tl-row {
  display: grid;
  grid-template-columns: 15px minmax(0, 1fr) 64px 26px 92px 110px 56px;
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

.atl-tl-status {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: center;
  font-size: var(--fs-sm);
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

.atl-tl-empty {
  padding: 8px;
  font-size: var(--fs-sm);
  color: var(--c-muted);
}
</style>
