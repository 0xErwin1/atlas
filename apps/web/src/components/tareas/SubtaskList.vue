<script setup lang="ts">
import { computed, ref } from 'vue';
import Avatar from '@/components/ui/Avatar.vue';
import Chip from '@/components/ui/Chip.vue';
import Icon from '@/components/ui/Icon.vue';
import { useLabelColorsStore } from '@/stores/labelColors';
import type { SubtaskDto } from '@/stores/taskDetail';

interface ColumnRef {
  id: string;
  name: string;
}

const props = defineProps<{
  subtasks: SubtaskDto[];
  columns: ColumnRef[];
}>();

const emit = defineEmits<{
  add: [title: string];
  promote: [readableId: string];
  open: [readableId: string];
  /** Toggle done / move a sub-task to a column via its checkbox. */
  setColumn: [readableId: string, columnId: string];
}>();

const labelColors = useLabelColorsStore();
const draft = ref('');

const columnName = (columnId: string): string =>
  props.columns.find((c) => c.id === columnId)?.name ?? '—';

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
  return bucket(columnName(sub.column_id)) === 'done';
}

const doneCount = computed(() => props.subtasks.filter(isDone).length);

function toggleDone(sub: SubtaskDto): void {
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
</script>

<template>
  <section>
    <div class="atl-sub-head">Sub-tasks · {{ doneCount }} / {{ subtasks.length }}</div>

    <div v-for="sub in subtasks" :key="sub.id" class="group atl-sub-row" :data-subtask="sub.id">
      <button
        type="button"
        class="atl-sub-check"
        :class="{ done: isDone(sub) }"
        :title="isDone(sub) ? 'Mark not done' : 'Mark done'"
        :aria-pressed="isDone(sub)"
        @click="toggleDone(sub)"
      >
        <Icon v-if="isDone(sub)" name="check" :size="12" :stroke-width="2.4" />
      </button>

      <button
        type="button"
        class="atl-sub-title"
        :class="{ done: isDone(sub) }"
        :data-subtask-open="sub.id"
        :title="`Open ${sub.readable_id}`"
        @click="emit('open', sub.readable_id)"
      >
        {{ sub.title }}
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
        :style="{ color: PILL[bucket(columnName(sub.column_id))].fg, background: PILL[bucket(columnName(sub.column_id))].bg }"
      >
        <span class="atl-sub-dot" :style="{ background: PILL[bucket(columnName(sub.column_id))].fg }" />
        {{ columnName(sub.column_id) }}
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
        placeholder="Add a sub-task…"
        class="atl-sub-add"
        @keydown.enter.prevent="submitDraft"
        @blur="submitDraft"
      />
    </div>
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

.atl-sub-check {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  flex: 0 0 auto;
  padding: 0;
  border: 1.5px solid var(--c-muted);
  border-radius: 3px;
  background: transparent;
  cursor: pointer;
}

.atl-sub-check.done {
  border-color: var(--c-success);
  background: var(--c-success);
  color: var(--c-background);
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
</style>
