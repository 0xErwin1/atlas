<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import ActivityComments from '@/components/tareas/ActivityComments.vue';
import AssigneeList from '@/components/tareas/AssigneeList.vue';
import AttachmentList from '@/components/tareas/AttachmentList.vue';
import Checklist from '@/components/tareas/Checklist.vue';
import CustomFieldsSection from '@/components/tareas/CustomFieldsSection.vue';
import ReferenceAdd from '@/components/tareas/ReferenceAdd.vue';
import ReferenceList from '@/components/tareas/ReferenceList.vue';
import SubtaskList from '@/components/tareas/SubtaskList.vue';
import TaskDescription from '@/components/tareas/TaskDescription.vue';
import Chip from '@/components/ui/Chip.vue';
import DatePicker from '@/components/ui/DatePicker.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import TagInput from '@/components/ui/TagInput.vue';
import { useInlineEdit } from '@/composables/useInlineEdit';
import type { AiAction } from '@/lib/aiPrompt';
import { filesFromClipboard, filesFromDataTransfer } from '@/lib/fileTransfer';
import { swatchById } from '@/lib/swatches';
import { useBoardsStore } from '@/stores/boards';
import { useLabelColorsStore } from '@/stores/labelColors';
import { useTagsStore } from '@/stores/tags';
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
    /** Render the Activity + Comments feed inline. Off when a host renders it in
     * a side rail/inspector instead (full view, dock, modal), so it is not
     * duplicated. */
    showSecondary?: boolean;
    /** Render the References section inline. Off only on the full desktop view,
     * whose inspector owns References; the dock and modal have no such surface, so
     * they keep it inline (default on). */
    showReferences?: boolean;
  }>(),
  { layout: 'wide', showSecondary: true, showReferences: true },
);

// Opening a sub-task is host-dependent: the full-screen route navigates, while the
// dock/modal pane reopens the sub-task in place. TaskBody therefore delegates
// upward instead of routing itself.
const emit = defineEmits<{ 'open-subtask': [readableId: string] }>();

const boards = useBoardsStore();
const tasks = useTasksStore();
const detail = useTaskDetailStore();
const tagsStore = useTagsStore();
const labelColors = useLabelColorsStore();
const workspace = useWorkspaceStore();
const ui = useUiStore();

const wide = computed(() => props.layout === 'wide');

// Kanban-summary fields (title, priority, status) reflect context-menu edits made
// on the board immediately; the full task supplies everything else. Prefer the
// summary when present, falling back to the loaded task on the standalone route.
const summary = computed(() => boards.findTaskByReadableId(props.task.readable_id) ?? null);

const title = computed(() => summary.value?.title ?? props.task.title);
const priority = computed(() => summary.value?.priority ?? props.task.priority ?? null);
const columnId = computed(() => summary.value?.column_id ?? props.task.column_id);

const statusName = computed(() => boards.columns.find((c) => c.id === columnId.value)?.name ?? null);

// The status pill is glazed with the user's registry color for this column value,
// mirroring how chips derive their swatch — never inferred from the status text.
const statusSwatch = computed(() => swatchById(labelColors.colorFor(`status:${columnId.value}`)));

// `<input type="date">` wants YYYY-MM-DD; the API stores a full ISO datetime.
const dueInputValue = computed(() => {
  const raw = props.task.due_date;
  if (raw == null) return '';
  const d = new Date(raw);
  return Number.isNaN(d.getTime()) ? '' : d.toISOString().slice(0, 10);
});

const PRIORITY_OPTIONS: DropdownOption[] = [
  { value: '', label: 'None', icon: 'flag', iconColor: 'var(--c-muted)' },
  { value: 'urgent', label: 'Urgent', icon: 'flag', iconColor: 'var(--c-danger)' },
  { value: 'high', label: 'High', icon: 'flag', iconColor: 'var(--c-primary)' },
  { value: 'medium', label: 'Medium', icon: 'flag', iconColor: 'var(--c-info)' },
  { value: 'low', label: 'Low', icon: 'flag', iconColor: 'var(--c-muted)' },
];

const statusOptions = computed<DropdownOption[]>(() =>
  boards.columns.map((c) => ({
    value: c.id,
    label: c.name,
    dot: swatchById(labelColors.colorFor(`status:${c.id}`)).fg,
  })),
);

const assignableOptions = computed<DropdownOption[]>(() => {
  const assigned = new Set(detail.assignees.map((a) => a.assignee.id));
  return workspace.members
    .filter((m) => !assigned.has(m.id))
    .map((m) => ({
      value: `${m.principal_type}:${m.id}`,
      label: m.display,
      icon: m.principal_type === 'api_key' ? 'sparkles' : 'user',
      iconColor: m.principal_type === 'api_key' ? 'var(--c-agent)' : 'var(--c-muted)',
    }));
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

function retryDetail(): void {
  const workspaceId = workspace.workspaceIdForSlug(props.ws);
  void detail.loadAll(props.ws, props.task.readable_id, workspaceId ?? undefined, props.task.id);
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

const labelsModel = computed<string[]>({
  get: () => props.task.labels ?? [],
  set: (next) => {
    void commitLabels(next);
  },
});

// Autocomplete pool: the workspace tag registry unioned with tags the app has
// already seen in loaded data, deduped case-insensitively.
const tagSuggestions = computed<string[]>(() => {
  const byLower = new Map<string, string>();
  for (const name of [...tagsStore.names, ...labelColors.tagNames]) {
    const key = name.trim().toLowerCase();
    if (key !== '' && !byLower.has(key)) byLower.set(key, name.trim());
  }
  return [...byLower.values()].sort((a, b) => a.localeCompare(b));
});

function tagColor(tag: string): string {
  return labelColors.colorFor(`tag:${tag.toLowerCase()}`);
}

function onRecolorTag(tag: string, swatchId: string): void {
  labelColors.setColor(`tag:${tag.toLowerCase()}`, swatchId);
}

function onCreateTag(name: string): void {
  void tagsStore.ensure(props.ws, name);
}

onMounted(() => {
  void tagsStore.load(props.ws);
  document.addEventListener('paste', onPaste);
});

onBeforeUnmount(() => {
  document.removeEventListener('paste', onPaste);
});

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
  emit('open-subtask', readableId);
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

const statusOpen = ref(false);
const addStatusOpen = ref(false);

async function onCreateStatus(value: string): Promise<void> {
  addStatusOpen.value = false;

  const name = value.trim();
  if (name === '') return;

  const created = await boards.createColumn(props.ws, props.task.board_id, name);
  if (created === null) {
    fail(boards.error);
    return;
  }

  await onChangeStatus(created.id);
}

function openAskAi(action: AiAction): void {
  ui.openAskAi(props.task, statusName.value, action);
}

const fileInput = ref<HTMLInputElement | null>(null);
const uploading = ref(false);
const dragActive = ref(false);

function onAttachClick(): void {
  fileInput.value?.click();
}

async function onFileSelected(event: Event): Promise<void> {
  const input = event.target as HTMLInputElement;
  const files = Array.from(input.files ?? []);
  input.value = '';

  await uploadFiles(files);
}

/**
 * Uploads each file as a task attachment, the same path the "Attach file" button
 * uses. Shared by the button, drag-and-drop and clipboard paste; reports one
 * banner for the batch and stops the spinner even if a file fails.
 */
async function uploadFiles(files: File[]): Promise<void> {
  if (files.length === 0) return;

  uploading.value = true;

  let uploaded = 0;
  for (const file of files) {
    const ok = await detail.uploadAttachment(props.ws, props.task.readable_id, file);
    if (ok) uploaded += 1;
    else fail(detail.error);
  }

  uploading.value = false;

  if (uploaded > 0) {
    ui.showBanner(uploaded === 1 ? 'Attachment uploaded' : `${uploaded} attachments uploaded`, 'success');
  }
}

function onDrop(event: DragEvent): void {
  dragActive.value = false;
  void uploadFiles(filesFromDataTransfer(event.dataTransfer));
}

// A file paste anywhere in the open task attaches it, mirroring the drop flow.
// Text pastes carry no files, so normal editing is never intercepted.
function onPaste(event: ClipboardEvent): void {
  const files = filesFromClipboard(event.clipboardData);
  if (files.length === 0) return;

  event.preventDefault();
  void uploadFiles(files);
}

async function onRemoveAttachment(attachmentId: string): Promise<void> {
  const ok = await detail.removeAttachment(props.ws, props.task.readable_id, attachmentId);
  if (!ok) fail(detail.error);
}

async function onChecklistToggle(itemId: string): Promise<void> {
  const ok = await detail.toggleChecklistItem(props.ws, props.task.readable_id, itemId);
  if (!ok) fail(detail.error);
}

async function onChecklistEdit(itemId: string, title: string): Promise<void> {
  const ok = await detail.updateChecklistItem(props.ws, props.task.readable_id, itemId, title);
  if (!ok && detail.error !== null) fail(detail.error);
}

async function onChecklistRemove(itemId: string): Promise<void> {
  const ok = await detail.removeChecklistItem(props.ws, props.task.readable_id, itemId);
  if (!ok) fail(detail.error);
}

async function onChecklistAdd(title: string): Promise<void> {
  const ok = await detail.addChecklistItem(props.ws, props.task.readable_id, title);
  if (!ok) fail(detail.error);
}

async function onChecklistPromote(itemId: string, columnId: string): Promise<void> {
  const boardId = boards.board?.id;
  if (boardId === undefined) return;

  const result = await detail.promoteChecklistItem(
    props.ws,
    props.task.readable_id,
    itemId,
    boardId,
    columnId,
  );

  if (result.ok && result.readableId !== undefined) {
    ui.showBanner(`${result.readableId} promoted to a board task`, 'success');
  } else {
    fail(detail.error);
  }
}
</script>

<template>
  <div
    class="atl-tv-body"
    :class="{ wide, 'drag-active': dragActive }"
    @dragover.prevent="dragActive = true"
    @dragleave="dragActive = false"
    @drop.prevent="onDrop"
  >
    <div v-if="dragActive" class="atl-tv-drophint">
      <Icon name="paperclip" :size="16" style="color: var(--c-primary);" />
      Drop to attach
    </div>
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

    <div class="atl-tv-agenthint">
      <Icon name="sparkles" :size="15" style="color: var(--c-agent); flex: 0 0 auto;" />
      <span>
        Ask <b style="color: var(--c-agent);">AI</b> to
        <a class="atl-tv-link" @click="openAskAi('summarize')">summarize</a>,
        <a class="atl-tv-link" @click="openAskAi('subtasks')">generate sub-tasks</a>,
        <a class="atl-tv-link" @click="openAskAi('similar')">find similar tasks</a>, or
        <a class="atl-tv-link" @click="openAskAi('start')">start it</a>
      </span>
    </div>

    <div class="atl-tv-fields" :class="{ wide }">
      <div class="atl-tv-col">
        <div class="atl-tv-field">
          <span class="atl-tv-label"><Icon name="circle-dot" :size="14" />Status</span>
          <span class="atl-tv-value">
            <Popover v-model:open="statusOpen" placement="bottom-start" width="200px">
              <template #trigger="{ toggle }">
                <button
                  type="button"
                  class="atl-tv-statuspill"
                  :style="{ color: statusSwatch.fg, background: statusSwatch.bg }"
                  @click="toggle"
                >
                  <span class="atl-tv-statusdot" :style="{ background: statusSwatch.fg }" />
                  {{ statusName ?? '—' }}
                  <Icon name="chevron-down" :size="12" />
                </button>
              </template>
              <template #default="{ close }">
                <div role="listbox" style="padding: 3px;">
                  <button
                    v-for="opt in statusOptions"
                    :key="opt.value"
                    type="button"
                    role="option"
                    :aria-selected="opt.value === columnId"
                    class="atl-mi"
                    style="width: 100%; border: none; background: transparent; text-align: left; gap: 8px;"
                    @click="onChangeStatus(opt.value); close()"
                  >
                    <span
                      :style="{ width: '7px', height: '7px', borderRadius: 'var(--r-full)', background: opt.dot, flex: '0 0 auto' }"
                    />
                    {{ opt.label }}
                  </button>

                  <div style="height: 1px; margin: 3px 0; background: var(--c-border);" />

                  <button
                    type="button"
                    class="atl-mi"
                    style="width: 100%; border: none; background: transparent; text-align: left; gap: 8px; color: var(--c-muted);"
                    @click="addStatusOpen = true; close()"
                  >
                    <Icon name="plus" :size="13" style="flex: 0 0 auto;" />
                    New status
                  </button>
                </div>
              </template>
            </Popover>
          </span>
        </div>
        <div class="atl-tv-field">
          <span class="atl-tv-label"><Icon name="users" :size="14" />Assignees</span>
          <span class="atl-tv-value" style="flex-direction: column; align-items: flex-start;">
            <LoadingState
              v-if="
                detail.collectionStatus.assignees === 'pending' &&
                detail.assignees.length === 0 &&
                !detail.collectionLoaded.assignees
              "
              label="Loading assignees…"
            />
            <ErrorState
              v-else-if="detail.collectionStatus.assignees === 'error'"
              title="Could not load assignees"
              :hint="detail.collectionErrors.assignees ?? undefined"
              @retry="retryDetail"
            />
            <template v-else>
              <AssigneeList :assignees="detail.assignees" @remove="onRemoveAssignee" />
              <Dropdown
                v-if="assignableOptions.length"
                :options="assignableOptions"
                placeholder="+ Add assignee"
                @change="onAddAssignee"
              />
            </template>
          </span>
        </div>
        <div class="atl-tv-field">
          <span class="atl-tv-label"><Icon name="calendar" :size="14" />Dates</span>
          <span class="atl-tv-value">
            <DatePicker
              :model-value="dueInputValue"
              @update:model-value="(v: string) => onChangeDue(v)"
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
          <span class="atl-tv-label"><Icon name="clock" :size="14" />Time estimate</span>
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
          <span class="atl-tv-value">
            <TagInput
              v-model="labelsModel"
              :suggestions="tagSuggestions"
              :color-for="tagColor"
              :on-recolor="onRecolorTag"
              @create="onCreateTag"
            />
          </span>
        </div>
      </div>
    </div>

    <div class="atl-tv-divider" />

    <div class="atl-tv-section-label">Description</div>
    <TaskDescription :markdown="task.description" :ws="ws" :readable-id="task.readable_id" />

    <div style="margin-top: 22px;">
      <LoadingState
        v-if="
          detail.collectionStatus.subtasks === 'pending' &&
          detail.subtasks.length === 0 &&
          !detail.collectionLoaded.subtasks
        "
        label="Loading sub-tasks…"
      />
      <ErrorState
        v-else-if="detail.collectionStatus.subtasks === 'error'"
        title="Could not load sub-tasks"
        :hint="detail.collectionErrors.subtasks ?? undefined"
        @retry="retryDetail"
      />
      <SubtaskList
        v-else
        :subtasks="detail.subtasks"
        :columns="boards.columns"
        @add="onAddSubtask"
        @promote="onPromoteSubtask"
        @open="onOpenSubtask"
        @set-column="onSubtaskSetColumn"
      />
    </div>

    <div style="margin-top: 22px;">
      <LoadingState
        v-if="
          detail.collectionStatus.checklist === 'pending' &&
          detail.checklist.length === 0 &&
          !detail.collectionLoaded.checklist
        "
        label="Loading checklist…"
      />
      <ErrorState
        v-else-if="detail.collectionStatus.checklist === 'error'"
        title="Could not load checklist"
        :hint="detail.collectionErrors.checklist ?? undefined"
        @retry="retryDetail"
      />
      <Checklist
        v-else
        :items="detail.checklist"
        :columns="boards.columns"
        @toggle="onChecklistToggle"
        @edit="onChecklistEdit"
        @remove="onChecklistRemove"
        @add="onChecklistAdd"
        @promote="onChecklistPromote"
      />
    </div>

    <div style="margin-top: 22px;">
      <CustomFieldsSection :ws="ws" :task="task" />
    </div>

    <div style="margin-top: 22px;">
      <div class="atl-tv-section-label">Attachments</div>
      <LoadingState
        v-if="
          detail.collectionStatus.attachments === 'pending' &&
          detail.attachments.length === 0 &&
          !detail.collectionLoaded.attachments
        "
        label="Loading attachments…"
      />
      <ErrorState
        v-else-if="detail.collectionStatus.attachments === 'error'"
        title="Could not load attachments"
        :hint="detail.collectionErrors.attachments ?? undefined"
        @retry="retryDetail"
      />
      <AttachmentList
        v-else
        :attachments="detail.attachments"
        :ws="ws"
        :readable-id="task.readable_id"
        @remove="onRemoveAttachment"
      />
    </div>

    <input ref="fileInput" type="file" class="hidden" multiple @change="onFileSelected" />

    <div class="atl-tv-actions">
      <button type="button" class="atl-tv-attach" :disabled="uploading" @click="onAttachClick">
        <Icon name="paperclip" :size="13" style="color: var(--c-muted);" />{{ uploading ? 'Uploading…' : 'Attach file' }}
      </button>
      <span class="atl-tv-attach-hint">or drop / paste a file anywhere</span>
    </div>

    <template v-if="showReferences">
      <div style="margin-top: 22px;">
        <div class="atl-tv-section-label">References</div>
        <LoadingState
          v-if="
            detail.collectionStatus.references === 'pending' &&
            detail.references.length === 0 &&
            !detail.collectionLoaded.references
          "
          label="Loading references…"
        />
        <ErrorState
          v-if="detail.collectionStatus.references === 'error'"
          title="Could not load references"
          :hint="detail.collectionErrors.references ?? undefined"
          @retry="retryDetail"
        />
        <LoadingState
          v-if="
            detail.collectionStatus.backlinks === 'pending' &&
            detail.backlinks.length === 0 &&
            !detail.collectionLoaded.backlinks
          "
          label="Loading backlinks…"
        />
        <ErrorState
          v-if="detail.collectionStatus.backlinks === 'error'"
          title="Could not load backlinks"
          :hint="detail.collectionErrors.backlinks ?? undefined"
          @retry="retryDetail"
        />
        <ReferenceList
          v-if="
            detail.collectionStatus.references !== 'pending' ||
            detail.collectionStatus.backlinks !== 'pending' ||
            detail.references.length > 0 ||
            detail.backlinks.length > 0
          "
          :references="detail.references"
          :backlinks="detail.backlinks"
          @remove="onRemoveReference"
        />
        <ReferenceAdd :ws="ws" :current-readable-id="task.readable_id" @add="onAddReference" />
      </div>
    </template>

    <template v-if="showSecondary">
      <div style="margin-top: 22px;">
        <div class="atl-tv-section-label">Activity</div>
        <LoadingState
          v-if="
            detail.collectionStatus.activity === 'pending' &&
            detail.activity.length === 0 &&
            !detail.collectionLoaded.activity
          "
          label="Loading activity…"
        />
        <ErrorState
          v-if="detail.collectionStatus.activity === 'error'"
          title="Could not load activity"
          :hint="detail.collectionErrors.activity ?? undefined"
          @retry="retryDetail"
        />
        <LoadingState
          v-if="
            detail.collectionStatus.comments === 'pending' &&
            detail.comments.length === 0 &&
            !detail.collectionLoaded.comments
          "
          label="Loading comments…"
        />
        <ErrorState
          v-if="detail.collectionStatus.comments === 'error'"
          title="Could not load comments"
          :hint="detail.collectionErrors.comments ?? undefined"
          @retry="retryDetail"
        />
        <ActivityComments
          v-if="
            detail.collectionStatus.activity !== 'pending' ||
            detail.collectionStatus.comments !== 'pending' ||
            detail.activity.length > 0 ||
            detail.comments.length > 0
          "
          :ws="ws"
          :readable-id="task.readable_id"
        />
      </div>
    </template>

    <PromptDialog
      :open="addStatusOpen"
      title="New status"
      placeholder="Status name"
      confirm-label="Create status"
      @confirm="onCreateStatus"
      @cancel="addStatusOpen = false"
    />
  </div>
</template>

<style scoped>
.atl-tv-body {
  position: relative;
  padding: 4px 0 28px;
}

.atl-tv-body.wide {
  max-width: 760px;
  margin: 0 auto;
  padding: 8px 0 40px;
}

.atl-tv-body.drag-active {
  outline: 2px dashed var(--c-primary);
  outline-offset: 6px;
  border-radius: var(--r-md);
}

/* The hint must not intercept drag events, or moving over it fires dragleave and
   flickers the active state. */
.atl-tv-drophint {
  position: absolute;
  top: 8px;
  right: 0;
  z-index: 2;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 10px;
  pointer-events: none;
  background: var(--c-panel);
  border: 1px solid var(--c-primary);
  border-radius: var(--r-md);
  font-size: var(--fs-sm);
  color: var(--c-primary);
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
  font-size: 19px;
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
  font-size: 24px;
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
  font-size: 19px;
  font-weight: var(--fw-bold);
  letter-spacing: -0.01em;
  line-height: 1.2;
  color: var(--c-foreground);
  outline: none;
}

.atl-tv-title-input.wide {
  font-size: 24px;
}

.atl-tv-agenthint {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 9px 12px;
  margin-bottom: 20px;
  border-radius: 4px;
  background: var(--c-agent-bg);
  border: 1px solid var(--c-agent-border);
  font-size: 12.5px;
  color: var(--c-foreground);
}

.atl-tv-link {
  color: var(--c-agent);
  cursor: pointer;
}

.atl-tv-link:hover {
  text-decoration: underline;
}

.atl-tv-fields {
  display: flex;
  flex-wrap: wrap;
  column-gap: 0;
  margin-bottom: 18px;
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
  width: 132px;
  flex: 0 0 132px;
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
  height: 28px;
  padding: 0 10px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  color: var(--c-foreground);
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  outline: none;
  transition:
    border-color 0.12s ease,
    box-shadow 0.12s ease;
}

.atl-tv-input:hover {
  border-color: var(--c-muted);
}

/* Numeric fields (estimate) carry no spin buttons — the value is typed, not nudged. */
.atl-tv-input[type='number'] {
  appearance: textfield;
  -moz-appearance: textfield;
}

.atl-tv-input[type='number']::-webkit-outer-spin-button,
.atl-tv-input[type='number']::-webkit-inner-spin-button {
  -webkit-appearance: none;
  appearance: none;
  margin: 0;
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

.atl-tv-statuspill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 24px;
  padding: 0 9px 0 8px;
  border: none;
  border-radius: 3px;
  font-size: var(--fs-sm);
  font-weight: var(--fw-semibold);
  cursor: pointer;
}

.atl-tv-statusdot {
  width: 7px;
  height: 7px;
  border-radius: var(--r-full);
  flex: 0 0 auto;
}

.atl-tv-actions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
  margin-top: 22px;
  margin-bottom: 24px;
}

/* A lone, low-emphasis affordance: dropping or pasting a file is the primary path
   (see the hint beside it), so the button stays small and borderless instead of
   reading as a call to action. */
.atl-tv-attach {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 24px;
  padding: 0 8px;
  margin-left: -8px;
  background: transparent;
  border: none;
  border-radius: var(--r-sm);
  color: var(--c-muted);
  font-size: var(--fs-sm);
  font-family: var(--font-ui);
  cursor: pointer;
  transition: color 0.12s ease, background 0.12s ease;
}

.atl-tv-attach:hover:not(:disabled) {
  color: var(--c-foreground);
  background: var(--c-raised);
}

.atl-tv-attach:disabled {
  cursor: default;
}

.atl-tv-attach-hint {
  font-size: var(--fs-xs);
  color: var(--c-muted);
}
</style>
