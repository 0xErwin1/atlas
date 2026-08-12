<script setup lang="ts">
import { computed, ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import SearchPicker from '@/components/tareas/SearchPicker.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import type { SearchHitDto } from '@/stores/search';

type CreateReferenceRequest = components['schemas']['CreateReferenceRequest'];

const props = withDefaults(
  defineProps<{
    ws: string;
    defaultKind?: string;
    large?: boolean;
    /** Readable ID of the task owning this picker, excluded so a task cannot
     * reference itself. */
    currentReadableId?: string;
  }>(),
  {
    defaultKind: 'relates',
    large: false,
  },
);

const emit = defineEmits<{ add: [body: CreateReferenceRequest] }>();

const KIND_OPTIONS: DropdownOption[] = [
  { value: 'relates', label: 'Relates to', icon: 'link' },
  { value: 'blocks', label: 'Blocks', icon: 'ban' },
  { value: 'parent', label: 'Parent', icon: 'git-branch' },
  { value: 'spec', label: 'Spec', icon: 'file-text' },
  { value: 'docs', label: 'Documentation', icon: 'book-text' },
];

const kind = ref(props.defaultKind);

// The server requires a document target for `spec`/`docs` and a task target for
// the others, so the picker only searches the valid target type for the kind.
const targetType = computed<'note' | 'task'>(() =>
  kind.value === 'spec' || kind.value === 'docs' ? 'note' : 'task',
);
const placeholder = computed(() => (targetType.value === 'note' ? 'Link a note…' : 'Link a task…'));

function pick(hit: SearchHitDto): void {
  const body: CreateReferenceRequest =
    hit.kind === 'task'
      ? { kind: kind.value, target_task_readable_id: hit.readable_id ?? null }
      : { kind: kind.value, target_document_id: hit.id };

  emit('add', body);
}
</script>

<template>
  <div class="atl-refadd" :class="{ lg: large }">
    <SearchPicker
      :ws="ws"
      :type="targetType"
      :placeholder="placeholder"
      :exclude-readable-id="currentReadableId"
      :large="large"
      :autofocus="large"
      @pick="pick"
    >
      <template #leading>
        <Dropdown :options="KIND_OPTIONS" :model-value="kind" @change="(v) => (kind = v)" />
      </template>
    </SearchPicker>
  </div>
</template>

<style scoped>
.atl-refadd {
  position: relative;
  margin-top: 8px;
}
</style>
