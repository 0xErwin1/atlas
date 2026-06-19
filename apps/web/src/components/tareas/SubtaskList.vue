<script setup lang="ts">
import { computed, ref } from 'vue';
import Avatar from '@/components/ui/Avatar.vue';
import Icon from '@/components/ui/Icon.vue';
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
}>();

const draft = ref('');

const columnName = (columnId: string): string => props.columns.find((c) => c.id === columnId)?.name ?? '—';

function submitDraft(): void {
  const title = draft.value.trim();
  if (title === '') return;
  emit('add', title);
  draft.value = '';
}
</script>

<template>
  <section>
    <div class="atl-sub-head">Sub-tasks · {{ subtasks.length }}</div>

    <div v-for="sub in subtasks" :key="sub.id" class="group atl-sub-row" :data-subtask="sub.id">
      <span class="atl-sub-status">{{ columnName(sub.column_id) }}</span>

      <button
        type="button"
        class="atl-sub-title"
        :data-subtask-open="sub.id"
        :title="`Open ${sub.readable_id}`"
        @click="emit('open', sub.readable_id)"
      >
        {{ sub.title }}
      </button>

      <span class="flex items-center" style="gap: 4px;">
        <Avatar
          v-for="a in sub.assignees ?? []"
          :key="`${a.type}:${a.id}`"
          :name="a.display_name ?? (a.type === 'api_key' ? 'Agent' : 'User')"
          :agent="a.type === 'api_key'"
          :size="16"
        />
      </span>

      <span v-if="sub.estimate != null" class="atl-sub-est">{{ sub.estimate }} pts</span>

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
  gap: 10px;
  padding: 5px 0;
  font-size: var(--fs-base);
}

.atl-sub-status {
  flex: 0 0 auto;
  padding: 1px 8px;
  border-radius: var(--r-full);
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  color: var(--c-muted);
  font-size: var(--fs-xs);
  white-space: nowrap;
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

.atl-sub-est {
  flex: 0 0 auto;
  color: var(--c-muted);
  font-size: var(--fs-xs);
  font-family: var(--font-mono);
}

.atl-sub-id {
  flex: 0 0 auto;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
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
  padding: 6px 0 0;
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
