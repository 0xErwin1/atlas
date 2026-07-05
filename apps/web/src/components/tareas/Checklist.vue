<script setup lang="ts">
import { computed, ref } from 'vue';
import { RouterLink } from 'vue-router';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import type { ChecklistItemDto } from '@/stores/taskDetail';

interface ColumnRef {
  id: string;
  name: string;
}

const props = defineProps<{
  items: ChecklistItemDto[];
  columns: ColumnRef[];
}>();

const emit = defineEmits<{
  toggle: [itemId: string];
  edit: [itemId: string, title: string];
  promote: [itemId: string, columnId: string];
  remove: [itemId: string];
  add: [title: string];
}>();

const doneCount = computed(() => props.items.filter((i) => i.checked).length);

const draft = ref('');
const pickerOpenForItem = ref<string | null>(null);

// Only one item is edited at a time; the input's mount focuses itself.
const editingId = ref<string | null>(null);
const editDraft = ref('');

function submitDraft(): void {
  const title = draft.value.trim();
  if (title === '') return;
  emit('add', title);
  draft.value = '';
}

function beginEdit(item: ChecklistItemDto): void {
  editingId.value = item.id;
  editDraft.value = item.title;
}

// Focus the edit input as it mounts. A function ref sidesteps the array a string
// ref would collect inside the `v-for`, and needs no post-render tick.
function focusOnMount(el: unknown): void {
  if (el instanceof HTMLInputElement) el.focus();
}

function commitEdit(): void {
  const id = editingId.value;
  if (id === null) return;

  const title = editDraft.value.trim();
  editingId.value = null;
  if (title !== '') emit('edit', id, title);
}

function cancelEdit(): void {
  editingId.value = null;
}

function openPicker(itemId: string): void {
  pickerOpenForItem.value = itemId;
}

function closePicker(): void {
  pickerOpenForItem.value = null;
}

function pickColumn(itemId: string, columnId: string): void {
  closePicker();
  emit('promote', itemId, columnId);
}
</script>

<template>
  <section>
    <div
      style="
        font-size: var(--fs-xs);
        font-weight: var(--fw-semibold);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        color: var(--c-muted);
        margin-bottom: 6px;
      "
    >
      Checklist · {{ doneCount }} / {{ items.length }}
    </div>

    <div
      v-for="item in items"
      :key="item.id"
      class="group flex items-center"
      style="gap: 8px; padding: 4px 0; font-size: var(--fs-base);"
      :data-checklist-item="item.id"
    >
      <button
        type="button"
        :aria-pressed="item.checked"
        :aria-label="item.checked ? 'Uncheck item' : 'Check item'"
        class="flex items-center justify-center shrink-0 cursor-pointer"
        :style="{
          width: '15px',
          height: '15px',
          borderRadius: 'var(--r-sm)',
          border: item.checked ? 'none' : '1px solid var(--c-muted)',
          backgroundColor: item.checked ? 'var(--c-success)' : 'transparent',
          color: 'var(--c-background)',
          padding: 0,
        }"
        @click="emit('toggle', item.id)"
      >
        <Icon v-if="item.checked" name="check" :size="12" />
      </button>

      <input
        v-if="editingId === item.id"
        :ref="focusOnMount"
        v-model="editDraft"
        type="text"
        class="atl-checklist-add"
        aria-label="Edit item"
        @keydown.enter.prevent="commitEdit"
        @keydown.esc.prevent="cancelEdit"
        @blur="commitEdit"
      />
      <span
        v-else
        class="flex-1 min-w-0 cursor-text"
        :style="{
          color: item.checked ? 'var(--c-muted)' : 'var(--c-foreground)',
          textDecoration: item.checked ? 'line-through' : 'none',
        }"
        @dblclick="beginEdit(item)"
      >
        {{ item.title }}
      </span>

      <button
        v-if="editingId !== item.id"
        type="button"
        title="Edit item"
        aria-label="Edit item"
        class="shrink-0 cursor-pointer opacity-0 group-hover:opacity-100 flex items-center justify-center"
        style="
          width: 22px;
          height: 22px;
          border: 1px solid var(--c-border);
          border-radius: var(--r-sm);
          background: var(--c-secondary);
          color: var(--c-muted);
        "
        @click="beginEdit(item)"
      >
        <Icon name="pencil" :size="13" />
      </button>

      <RouterLink
        v-if="item.promoted_readable_id"
        :to="{ name: 'task-detail', params: { readableId: item.promoted_readable_id } }"
        class="atl-checklist-promoted shrink-0"
        :title="`Open ${item.promoted_readable_id}`"
      >
        {{ item.promoted_readable_id }}
      </RouterLink>

      <Popover
        v-else-if="columns.length > 0"
        :open="pickerOpenForItem === item.id"
        placement="bottom-end"
        width="160px"
        @update:open="(v) => { if (!v) closePicker(); }"
      >
        <template #trigger>
          <button
            type="button"
            title="Promote to task"
            aria-label="Promote to task"
            class="shrink-0 cursor-pointer opacity-0 group-hover:opacity-100 flex items-center justify-center"
            style="
              width: 22px;
              height: 22px;
              border: 1px solid var(--c-border);
              border-radius: var(--r-sm);
              background: var(--c-secondary);
              color: var(--c-muted);
            "
            @click="openPicker(item.id)"
          >
            <Icon name="arrow-up-right" :size="13" />
          </button>
        </template>

        <template #default="{ close }">
          <div role="listbox" style="padding: 3px;">
            <button
              v-for="col in columns"
              :key="col.id"
              type="button"
              role="option"
              :data-column-id="col.id"
              class="atl-mi"
              style="width: 100%; border: none; background: transparent; text-align: left; gap: 8px;"
              @click="pickColumn(item.id, col.id); close()"
            >
              {{ col.name }}
            </button>
          </div>
        </template>
      </Popover>

      <button
        type="button"
        title="Delete item"
        aria-label="Delete item"
        class="shrink-0 cursor-pointer opacity-0 group-hover:opacity-100 flex items-center justify-center"
        style="
          width: 22px;
          height: 22px;
          border: 1px solid var(--c-border);
          border-radius: var(--r-sm);
          background: var(--c-secondary);
          color: var(--c-muted);
        "
        @click="emit('remove', item.id)"
      >
        <Icon name="trash" :size="13" />
      </button>
    </div>

    <div class="flex items-center" style="gap: 8px; padding: 6px 0 0;">
      <Icon name="plus" :size="13" style="color: var(--c-muted); flex: 0 0 auto;" />
      <input
        v-model="draft"
        type="text"
        placeholder="Add an item…"
        class="atl-checklist-add"
        @keydown.enter.prevent="submitDraft"
        @blur="submitDraft"
      />
    </div>
  </section>
</template>

<style scoped>
.atl-checklist-add {
  flex: 1;
  min-width: 0;
  background: transparent;
  border: none;
  outline: none;
  color: var(--c-foreground);
  font-family: var(--font-ui);
  font-size: var(--fs-base);
}

.atl-checklist-add::placeholder {
  color: var(--c-muted);
}

.atl-checklist-promoted {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--c-info);
  cursor: pointer;
}

.atl-checklist-promoted:hover {
  text-decoration: underline;
}
</style>
