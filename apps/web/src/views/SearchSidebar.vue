<script setup lang="ts">
import { computed } from 'vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import Icon from '@/components/ui/Icon.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import type { SearchType } from '@/stores/search';
import { useSearchStore } from '@/stores/search';

const props = defineProps<{
  query: string;
}>();

const emit = defineEmits<{
  (e: 'input', value: string): void;
  (e: 'clear'): void;
  (e: 'rerun'): void;
}>();

const store = useSearchStore();

const typeOptions: DropdownOption[] = [
  { value: 'all', label: 'All types' },
  { value: 'note', label: 'Notes' },
  { value: 'task', label: 'Tasks' },
];

const queryModel = computed(() => props.query);

function onType(value: string): void {
  store.setType(value as SearchType);
  emit('rerun');
}
</script>

<template>
  <div>
    <div :style="{ padding: '8px 10px' }">
      <div
        class="flex items-center gap-2"
        :style="{
          height: '28px',
          padding: '0 9px',
          background: 'var(--c-input)',
          border: '1px solid var(--c-border)',
          borderRadius: 'var(--r-sm)',
        }"
      >
        <Icon name="search" :size="13" :style="{ color: 'var(--c-muted)' }" />
        <input
          type="text"
          placeholder="Search…"
          :value="queryModel"
          :style="{
            flex: 1,
            minWidth: 0,
            background: 'transparent',
            border: 'none',
            outline: 'none',
            color: 'var(--c-foreground)',
            fontSize: 'var(--fs-base)',
          }"
          @input="emit('input', ($event.target as HTMLInputElement).value)"
        >
        <button
          v-if="queryModel"
          type="button"
          aria-label="Clear search"
          class="inline-flex items-center cursor-pointer"
          :style="{ color: 'var(--c-muted)', border: 'none', background: 'transparent' }"
          @click="emit('clear')"
        >
          <Icon name="x" :size="13" />
        </button>
      </div>
    </div>

    <div :style="{ padding: '0 10px 4px' }">
      <SectionLabel flush>Type</SectionLabel>
      <Dropdown
        :options="typeOptions"
        :model-value="store.type"
        :style="{ display: 'flex', width: '100%' }"
        @change="onType"
      />
    </div>
  </div>
</template>
