<script setup lang="ts">
import { computed, ref } from 'vue';
import SearchPicker from '@/components/tareas/SearchPicker.vue';
import Avatar from '@/components/ui/Avatar.vue';
import Chip from '@/components/ui/Chip.vue';
import Icon from '@/components/ui/Icon.vue';
import SquareCheckbox from '@/components/ui/SquareCheckbox.vue';
import { useLabelColorsStore } from '@/stores/labelColors';
import type { SearchHitDto } from '@/stores/search';
import type { SubtaskDto } from '@/stores/taskDetail';

interface ColumnRef {
  id: string;
  name: string;
}

const props = defineProps<{
  ws: string;
  subtasks: SubtaskDto[];
  columns: ColumnRef[];
  /** Board of the parent task; `columns` belong to it. */
  boardId: string;
  /** Readable ID of the parent, excluded from the attach search. */
  parentReadableId: string;
}>();

const emit = defineEmits<{
  add: [title: string];
  /** Convert an existing task into a sub-task of this task. */
  attach: [readableId: string];
  promote: [readableId: string];
  open: [readableId: string];
  /** Toggle done / move a sub-task to a column via its checkbox. */
  setColumn: [readableId: string, columnId: string];
}>();

const labelColors = useLabelColorsStore();
const draft = ref('');
const attaching = ref(false);

const columnName = (columnId: string): string => props.columns.find((c) => c.id === columnId)?.name ?? '—';

/**
 * A sub-task keeps its own board, so it may not live on the parent's. When it
 * does not, `columns` cannot name its column and its status is read from the
 * summary the server resolved.
 */
const isForeign = (sub: SubtaskDto): boolean => sub.board_id !== props.boardId;

const statusLabel = (sub: SubtaskDto): string =>
  isForeign(sub) ? (sub.column_name ?? '—') : columnName(sub.column_id);

// Map a column name to a semantic bucket so the status pill and the done checkbox
// follow the board's flow (matching the kanban column dots). Structural, not a
// free-text value, so a name-based mapping is appropriate here.
type Bucket = 'todo' | 'progress' | 'review' | 'done';

function bucket(name: string): Bucket {
  const n = name.toLowerCase();
  if (/(done|complete|closed|shipped|merged)/.test(n)) return 'done';
  if (/(review|qa|verify)/.test(n)) return 'review';
  if (/(progress|doing|active|wip)/.test(n)) return 'progress';
  return 'todo';
}

const PILL: Record<Bucket, { fg: string; bg: string }> = {
  done: { fg: 'var(--c-success)', bg: 'rgba(170, 217, 76, 0.13)' },
  review: { fg: 'var(--c-primary)', bg: 'rgba(255, 180, 84, 0.13)' },
  progress: { fg: 'var(--c-info)', bg: 'rgba(89, 194, 255, 0.13)' },
  todo: { fg: 'var(--c-muted)', bg: 'rgba(179, 177, 173, 0.1)' },
};

const doneColumnId = computed(
  () => props.columns.find((c) => bucket(c.name) === 'done')?.id ?? props.columns.at(-1)?.id ?? null,
);
const todoColumnId = computed(() => props.columns[0]?.id ?? null);

function isDone(sub: SubtaskDto): boolean {
  return bucket(statusLabel(sub)) === 'done';
}

const doneCount = computed(() => props.subtasks.filter(isDone).length);

function toggleDone(sub: SubtaskDto): void {
  // Only the parent board's columns are known here, so toggling a sub-task that
  // lives elsewhere would silently move it across boards.
  if (isForeign(sub)) return;

  const target = isDone(sub) ? todoColumnId.value : doneColumnId.value;
  if (target === null || target === sub.column_id) return;
  emit('setColumn', sub.readable_id, target);
}

function submitDraft(): void {
  const title = draft.value.trim();
  if (title === '') return;
  emit('add', title);
  draft.value = '';
}

function attach(hit: SearchHitDto): void {
  if (hit.readable_id == null) return;
  emit('attach', hit.readable_id);
  attaching.value = false;
}
</script>

<template>
  <section>
    <div class="atl-sub-head">Sub-tasks · {{ doneCount }} / {{ subtasks.length }}</div>

    <div v-for="sub in subtasks" :key="sub.id" class="group atl-sub-row" :data-subtask="sub.id">
      <SquareCheckbox
        v-if="!isForeign(sub)"
        :model-value="isDone(sub)"
        :label="isDone(sub) ? `Mark ${sub.title} not done` : `Mark ${sub.title} done`"
        tone="success"
        :title="isDone(sub) ? 'Mark not done' : 'Mark done'"
        @update:model-value="toggleDone(sub)"
      />
      <span
        v-else
        class="atl-sub-foreign"
        :title="`On the ${sub.board_name} board — open it to change its status`"
      >
        <Icon name="square-kanban" :size="11" />
      </span>

      <button
        type="button"
        class="atl-sub-title"
        :class="{ done: isDone(sub) }"
        :data-subtask-open="sub.id"
        :title="`Open ${sub.readable_id}`"
        @click="emit('open', sub.readable_id)"
      >
        <span :title="sub.title">{{ sub.title }}</span>
      </button>

      <Chip
        v-for="label in sub.labels ?? []"
        :key="label"
        :color="labelColors.colorFor(`tag:${label.toLowerCase()}`)"
      >
        {{ label }}
      </Chip>

      <span v-if="sub.estimate != null" class="atl-sub-est">{{ sub.estimate }} pts</span>

      <span
        class="atl-sub-status"
        :style="{ color: PILL[bucket(statusLabel(sub))].fg, background: PILL[bucket(statusLabel(sub))].bg }"
        :title="isForeign(sub) ? `${sub.board_name} · ${statusLabel(sub)}` : statusLabel(sub)"
      >
        <span class="atl-sub-dot" :style="{ background: PILL[bucket(statusLabel(sub))].fg }" />
        {{ statusLabel(sub) }}
      </span>

      <Avatar
        v-if="(sub.assignees ?? []).length > 0"
        :name="sub.assignees?.[0]?.display_name ?? ''"
        :agent="sub.assignees?.[0]?.type === 'api_key'"
        :size="18"
      />
      <span v-else class="atl-sub-unassigned" title="Unassigned">
        <Icon name="user" :size="11" />
      </span>

      <span class="atl-sub-id">{{ sub.readable_id }}</span>

      <button
        type="button"
        class="atl-sub-promote opacity-0 group-hover:opacity-100"
        :data-subtask-promote="sub.id"
        title="Promote to a board task"
        aria-label="Promote to a board task"
        @click="emit('promote', sub.readable_id)"
      >
        <Icon name="arrow-up-right" :size="13" />
      </button>
    </div>

    <div class="atl-sub-add-row">
      <Icon name="plus" :size="13" style="color: var(--c-muted); flex: 0 0 auto;" />
      <input
        v-model="draft"
        type="text"
        name="atlas-subtask-title"
        autocomplete="off"
        data-form-type="other"
        data-1p-ignore
        data-lpignore="true"
        data-bwignore
        placeholder="Add a sub-task…"
        class="atl-sub-add"
        aria-label="New sub-task title"
        @keydown.enter.prevent="submitDraft"
        @blur="submitDraft"
      />

      <button
        type="button"
        class="atl-sub-attach"
        data-subtask-attach
        :title="attaching ? 'Cancel' : 'Convert an existing task into a sub-task'"
        @click="attaching = !attaching"
      >
        <Icon :name="attaching ? 'x' : 'link'" :size="12" />
        {{ attaching ? 'Cancel' : 'Link existing' }}
      </button>
    </div>

    <SearchPicker
      v-if="attaching"
      :ws="ws"
      type="task"
      placeholder="Search a task to turn into a sub-task…"
      :exclude-readable-id="parentReadableId"
      autofocus
      @pick="attach"
    />
  </section>
</template>

<style scoped>
.atl-sub-head {
  font-size: var(--fs-xs);
  font-weight: var(--fw-semibold);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--c-muted);
  margin-bottom: 6px;
}

.atl-sub-row {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 7px 8px;
  border-radius: var(--r-lg);
  font-size: var(--fs-base);
}

.atl-sub-row:hover {
  background: rgba(179, 177, 173, 0.05);
}

.atl-sub-title {
  flex: 1;
  min-width: 0;
  text-align: left;
  background: transparent;
  border: none;
  padding: 0;
  cursor: pointer;
  color: var(--c-foreground);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-sub-title:hover {
  color: var(--c-primary);
  text-decoration: underline;
}

.atl-sub-title.done {
  color: var(--c-muted);
  text-decoration: line-through;
}

.atl-sub-est {
  flex: 0 0 auto;
  color: var(--c-muted);
  font-size: var(--fs-xs);
  font-family: var(--font-mono);
}

.atl-sub-status {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  flex: 0 0 auto;
  height: 20px;
  padding: 0 8px 0 7px;
  border-radius: var(--r-lg);
  font-size: 10.5px;
  font-weight: var(--fw-semibold);
  white-space: nowrap;
}

.atl-sub-dot {
  width: 6px;
  height: 6px;
  border-radius: var(--r-full);
  flex: 0 0 auto;
}

.atl-sub-unassigned {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  flex: 0 0 auto;
  border: 1px dashed var(--c-muted);
  border-radius: var(--r-sm);
  color: var(--c-muted);
}

.atl-sub-id {
  flex: 0 0 auto;
  width: 46px;
  text-align: right;
  font-family: var(--font-mono);
  font-size: 10.5px;
  color: var(--c-muted);
}

.atl-sub-promote {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  background: var(--c-secondary);
  color: var(--c-muted);
  cursor: pointer;
}

.atl-sub-add-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px;
}

.atl-sub-add {
  flex: 1;
  min-width: 0;
  background: transparent;
  border: none;
  outline: none;
  color: var(--c-foreground);
  font-family: var(--font-ui);
  font-size: var(--fs-base);
}

.atl-sub-add::placeholder {
  color: var(--c-muted);
}

.atl-sub-attach {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  flex: 0 0 auto;
  height: 22px;
  padding: 0 8px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  background: var(--c-secondary);
  color: var(--c-muted);
  font-size: var(--fs-xs);
  cursor: pointer;
}

.atl-sub-attach:hover {
  color: var(--c-foreground);
}

.atl-sub-foreign {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  flex: 0 0 auto;
  color: var(--c-muted);
}
</style>
