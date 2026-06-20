<script setup lang="ts">
import { computed, ref } from 'vue';
import { useRouter } from 'vue-router';
import type { components } from '@/api/types.d.ts';
import ActivityFeed from '@/components/tareas/ActivityFeed.vue';
import AssigneeList from '@/components/tareas/AssigneeList.vue';
import ReferenceAdd from '@/components/tareas/ReferenceAdd.vue';
import ReferenceList from '@/components/tareas/ReferenceList.vue';
import SubtaskList from '@/components/tareas/SubtaskList.vue';
import TaskDescription from '@/components/tareas/TaskDescription.vue';
import Chip from '@/components/ui/Chip.vue';
import CollapsibleText from '@/components/ui/CollapsibleText.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import Icon from '@/components/ui/Icon.vue';
import { useInlineEdit } from '@/composables/useInlineEdit';
import { useBoardsStore } from '@/stores/boards';
import { useLabelColorsStore } from '@/stores/labelColors';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useTasksStore } from '@/stores/tasks';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

type TaskDto = components['schemas']['TaskDto'];

const props = withDefaults(
  defineProps<{
    task: TaskDto;
    ws: string;
    layout?: 'wide' | 'narrow';
    /** Render References + Activity inline. Off when a host (the full view) shows
     * them in a side inspector instead, so they are not duplicated. */
    showSecondary?: boolean;
  }>(),
  { layout: 'wide', showSecondary: true },
);

const boards = useBoardsStore();
const tasks = useTasksStore();
const detail = useTaskDetailStore();
const labelColors = useLabelColorsStore();
const workspace = useWorkspaceStore();
const ui = useUiStore();
const router = useRouter();

const wide = computed(() => props.layout === 'wide');

// Kanban-summary fields (title, priority, status) reflect context-menu edits made
// on the board immediately; the full task supplies everything else. Prefer the
// summary when present, falling back to the loaded task on the standalone route.
const summary = computed(() => boards.findTaskByReadableId(props.task.readable_id) ?? null);

const title = computed(() => summary.value?.title ?? props.task.title);
const priority = computed(() => summary.value?.priority ?? props.task.priority ?? null);
const columnId = computed(() => summary.value?.column_id ?? props.task.column_id);

const statusName = computed(() => boards.columns.find((c) => c.id === columnId.value)?.name ?? null);

// `<input type="date">` wants YYYY-MM-DD; the API stores a full ISO datetime.
const dueInputValue = computed(() => {
  const raw = props.task.due_date;
  if (raw == null) return '';
  const d = new Date(raw);
  return Number.isNaN(d.getTime()) ? '' : d.toISOString().slice(0, 10);
});

const labelDraft = ref('');

const PRIORITY_OPTIONS: DropdownOption[] = [
  { value: '', label: 'None' },
  { value: 'urgent', label: 'Urgent' },
  { value: 'high', label: 'High' },
  { value: 'medium', label: 'Medium' },
  { value: 'low', label: 'Low' },
];

const statusOptions = computed<DropdownOption[]>(() =>
  boards.columns.map((c) => ({ value: c.id, label: c.name })),
);

const assignableOptions = computed<DropdownOption[]>(() => {
  const assigned = new Set(detail.assignees.map((a) => a.assignee.id));
  return workspace.members
    .filter((m) => !assigned.has(m.id))
    .map((m) => ({ value: `${m.principal_type}:${m.id}`, label: m.display }));
});

const {
  active: titleActive,
  value: titleValue,
  inputRef: titleInputRef,
  start: startTitle,
  commit: commitTitleEdit,
  onKeydown: onTitleKeydown,
} = useInlineEdit<string>((next, readableId) => {
  void commitTitle(readableId, next);
});

function fail(message: string | null): void {
  if (message) ui.showBanner(message, 'error');
}

async function commitTitle(readableId: string, next: string): Promise<void> {
  const ok = await boards.updateTask(props.ws, readableId, { title: next });
  if (ok) tasks.patchOpenTask({ title: next });
  else fail(boards.error);
}

async function onChangeStatus(value: string): Promise<void> {
  const ok = await boards.moveTaskToColumn(props.ws, props.task.readable_id, value);
  if (ok) tasks.patchOpenTask({ column_id: value });
  else fail(boards.error);
}

async function onChangePriority(value: string): Promise<void> {
  const next = value === '' ? null : value;
  const ok = await boards.updateTask(props.ws, props.task.readable_id, { priority: next });
  if (ok) tasks.patchOpenTask({ priority: next });
  else fail(boards.error);
}

async function onChangeDue(value: string): Promise<void> {
  const due = value === '' ? null : new Date(`${value}T00:00:00Z`).toISOString();
  const ok = await boards.updateTask(props.ws, props.task.readable_id, { due_date: due });
  if (ok) tasks.patchOpenTask({ due_date: due });
  else fail(boards.error);
}

async function onChangeEstimate(value: string): Promise<void> {
  const trimmed = value.trim();
  const estimate = trimmed === '' ? null : Number.parseInt(trimmed, 10);
  if (estimate !== null && (Number.isNaN(estimate) || estimate < 0)) return;
  const ok = await boards.updateTask(props.ws, props.task.readable_id, { estimate });
  if (ok) tasks.patchOpenTask({ estimate });
  else fail(boards.error);
}

async function commitLabels(labels: string[]): Promise<void> {
  const ok = await boards.updateTask(props.ws, props.task.readable_id, { labels });
  if (ok) tasks.patchOpenTask({ labels });
  else fail(boards.error);
}

function onAddLabel(): void {
  const value = labelDraft.value.trim();
  labelDraft.value = '';
  if (value === '') return;
  const current = props.task.labels ?? [];
  if (!current.includes(value)) void commitLabels([...current, value]);
}

function onRemoveLabel(label: string): void {
  void commitLabels((props.task.labels ?? []).filter((l) => l !== label));
}

async function onAddAssignee(ref: string): Promise<void> {
  const [assignee_type, assignee_id] = ref.split(':');
  if (assignee_type === undefined || assignee_id === undefined) return;
  const ok = await detail.addAssignee(props.ws, props.task.readable_id, { assignee_type, assignee_id });
  if (!ok) fail(detail.error);
}

async function onRemoveAssignee(assigneeType: string, assigneeId: string): Promise<void> {
  const ok = await detail.removeAssignee(props.ws, props.task.readable_id, assigneeType, assigneeId);
  if (!ok) fail(detail.error);
}

async function onAddSubtask(title: string): Promise<void> {
  const ok = await detail.addSubtask(props.ws, props.task.readable_id, title);
  if (!ok) fail(detail.error);
}

async function onPromoteSubtask(readableId: string): Promise<void> {
  const ok = await detail.promoteSubtask(props.ws, readableId);
  if (ok) ui.showBanner(`${readableId} promoted to a board task`, 'success');
  else fail(detail.error);
}

async function onSubtaskSetColumn(readableId: string, columnId: string): Promise<void> {
  const ok = await detail.moveSubtaskToColumn(props.ws, readableId, columnId);
  if (!ok) fail(detail.error);
}

function onOpenSubtask(readableId: string): void {
  void router.push({ name: 'task-detail', params: { readableId } });
}

async function onAddReference(body: components['schemas']['CreateReferenceRequest']): Promise<void> {
  const ok = await detail.addReference(props.ws, props.task.readable_id, body);
  if (ok) ui.showBanner('Reference added', 'success');
  else fail(detail.error);
}

async function onRemoveReference(referenceId: string): Promise<void> {
  const ok = await detail.removeReference(props.ws, props.task.readable_id, referenceId);
  if (!ok) fail(detail.error);
}
</script>

<template>
  <div class="atl-tv-body" :class="{ wide }">
    <div class="atl-tv-typebar">
      <span class="atl-tv-typechip">
        <Icon name="square-kanban" :size="13" style="color: var(--c-primary);" />
        Task
      </span>
      <span class="atl-tv-id">{{ task.readable_id }}</span>
      <span style="flex: 1;" />
      <Chip
        v-for="label in task.labels ?? []"
        :key="label"
        :color="labelColors.colorFor(`tag:${label.toLowerCase()}`)"
      >
        {{ label }}
      </Chip>
    </div>

    <input
      v-if="titleActive === task.readable_id"
      ref="titleInputRef"
      v-model="titleValue"
      class="atl-tv-title-input"
      :class="{ wide }"
      @keydown="onTitleKeydown"
      @blur="commitTitleEdit"
    />
    <h1
      v-else
      class="atl-tv-title"
      :class="{ wide }"
      title="Click to rename"
      @click="startTitle(task.readable_id, title, true)"
    >
      {{ title }}
    </h1>

    <div class="atl-tv-fields" :class="{ wide }">
      <div class="atl-tv-col">
        <div class="atl-tv-field">
          <span class="atl-tv-label"><Icon name="circle-dot" :size="14" />Status</span>
          <span class="atl-tv-value">
            <Dropdown :options="statusOptions" :model-value="columnId" placeholder="—" @change="onChangeStatus" />
          </span>
        </div>
        <div class="atl-tv-field">
          <span class="atl-tv-label"><Icon name="users" :size="14" />Assignees</span>
          <span class="atl-tv-value" style="flex-direction: column; align-items: flex-start;">
            <AssigneeList :assignees="detail.assignees" @remove="onRemoveAssignee" />
            <Dropdown
              v-if="assignableOptions.length"
              :options="assignableOptions"
              placeholder="+ Add assignee"
              @change="onAddAssignee"
            />
          </span>
        </div>
        <div class="atl-tv-field">
          <span class="atl-tv-label"><Icon name="calendar" :size="14" />Due</span>
          <span class="atl-tv-value">
            <input
              type="date"
              class="atl-tv-input"
              :value="dueInputValue"
              @change="onChangeDue(($event.target as HTMLInputElement).value)"
            />
          </span>
        </div>
      </div>

      <div class="atl-tv-col">
        <div class="atl-tv-field">
          <span class="atl-tv-label"><Icon name="flag" :size="14" />Priority</span>
          <span class="atl-tv-value">
            <Dropdown :options="PRIORITY_OPTIONS" :model-value="priority ?? ''" @change="onChangePriority" />
          </span>
        </div>
        <div class="atl-tv-field">
          <span class="atl-tv-label"><Icon name="clock" :size="14" />Estimate</span>
          <span class="atl-tv-value">
            <input
              type="number"
              min="0"
              class="atl-tv-input"
              style="width: 80px;"
              placeholder="—"
              :value="task.estimate ?? ''"
              @change="onChangeEstimate(($event.target as HTMLInputElement).value)"
            />
            <span style="color: var(--c-muted); font-size: var(--fs-xs);">pts</span>
          </span>
        </div>
        <div class="atl-tv-field">
          <span class="atl-tv-label"><Icon name="tag" :size="14" />Tags</span>
          <span class="atl-tv-value" style="flex-wrap: wrap;">
            <span v-for="label in task.labels ?? []" :key="label" class="atl-tag">
              {{ label }}
              <button
                type="button"
                class="atl-tag-x"
                aria-label="Remove tag"
                @click="onRemoveLabel(label)"
              >
                ×
              </button>
            </span>
            <input
              v-model="labelDraft"
              class="atl-tv-input"
              style="width: 96px;"
              placeholder="+ Tag"
              @keydown.enter.prevent="onAddLabel"
              @blur="onAddLabel"
            />
          </span>
        </div>
      </div>
    </div>

    <div class="atl-tv-divider" />

    <div class="atl-tv-section-label">Description</div>
    <CollapsibleText :collapsed-height="220">
      <TaskDescription :markdown="task.description" :ws="ws" :readable-id="task.readable_id" />
    </CollapsibleText>

    <div style="margin-top: 22px;">
      <SubtaskList
        :subtasks="detail.subtasks"
        :columns="boards.columns"
        @add="onAddSubtask"
        @promote="onPromoteSubtask"
        @open="onOpenSubtask"
        @set-column="onSubtaskSetColumn"
      />
    </div>

    <template v-if="showSecondary">
      <div style="margin-top: 22px;">
        <div class="atl-tv-section-label">References</div>
        <ReferenceList :references="detail.references" @remove="onRemoveReference" />
        <ReferenceAdd :ws="ws" @add="onAddReference" />
      </div>

      <div style="margin-top: 22px;">
        <div class="atl-tv-section-label">Activity</div>
        <ActivityFeed :items="detail.activity" />
      </div>
    </template>
  </div>
</template>

<style scoped>
.atl-tv-body {
  padding: 4px 0 28px;
}

.atl-tv-body.wide {
  max-width: 760px;
  margin: 0 auto;
  padding: 8px 0 40px;
}

.atl-tv-typebar {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
  flex-wrap: wrap;
}

.atl-tv-typechip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 22px;
  padding: 0 8px;
  border-radius: var(--r-sm);
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  font-size: 11.5px;
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-tv-id {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-tv-title {
  font-size: var(--fs-lg);
  font-weight: var(--fw-bold);
  line-height: 1.2;
  letter-spacing: -0.01em;
  color: var(--c-foreground);
  margin: 0 0 16px;
  padding: 2px 4px;
  margin-left: -4px;
  border-radius: var(--r-sm);
  cursor: text;
}

.atl-tv-title.wide {
  font-size: var(--fs-title);
}

.atl-tv-title:hover {
  background: var(--c-raised);
}

.atl-tv-title-input {
  width: 100%;
  margin: 0 0 16px;
  padding: 2px 4px;
  background: var(--c-panel);
  border: 1px solid var(--c-primary);
  border-radius: var(--r-sm);
  font-size: var(--fs-lg);
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
  outline: none;
}

.atl-tv-title-input.wide {
  font-size: var(--fs-title);
}

.atl-tv-fields {
  display: flex;
  flex-wrap: wrap;
  column-gap: 0;
  margin-bottom: 18px;
  padding: 8px 14px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg);
}

.atl-tv-fields.wide {
  column-gap: 36px;
}

.atl-tv-col {
  flex: 1 1 100%;
  min-width: 0;
}

.atl-tv-fields.wide .atl-tv-col {
  flex: 1 1 300px;
}

.atl-tv-field {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  min-height: 30px;
  padding: 4px 0;
  min-width: 0;
}

.atl-tv-label {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 92px;
  flex: 0 0 92px;
  padding-top: 3px;
  color: var(--c-muted);
  font-size: var(--fs-sm);
}

.atl-tv-value {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 7px;
  flex-wrap: wrap;
  font-size: var(--fs-sm);
  color: var(--c-foreground);
}

.atl-tv-value.empty {
  color: var(--c-muted);
}

.atl-tv-input {
  height: 26px;
  padding: 0 8px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  color: var(--c-foreground);
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  outline: none;
}

.atl-tv-input:focus {
  border-color: var(--c-primary);
}

.atl-tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 22px;
  padding: 0 4px 0 8px;
  border-radius: var(--r-sm);
  background: var(--c-selection);
  color: var(--c-foreground);
  font-size: var(--fs-xs);
}

.atl-tag-x {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
  font-size: 14px;
  line-height: 1;
}

.atl-tag-x:hover {
  color: var(--c-foreground);
}

.atl-tv-divider {
  height: 1px;
  background: var(--c-border);
  margin: 4px 0 18px;
}

.atl-tv-section-label {
  font-size: var(--fs-xs);
  font-weight: var(--fw-semibold);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--c-muted);
  margin-bottom: 8px;
}
</style>
