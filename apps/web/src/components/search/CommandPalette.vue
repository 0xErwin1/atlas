<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import Chip from '@/components/ui/Chip.vue';
import Icon from '@/components/ui/Icon.vue';
import Kbd from '@/components/ui/Kbd.vue';
import { filterLocalActions, type LocalAction, useSearch } from '@/composables/useSearch';
import { sanitizeSnippet } from '@/lib/sanitize';
import type { SearchHitDto } from '@/stores/search';

export type PaletteSelection = { type: 'action'; action: LocalAction } | { type: 'hit'; hit: SearchHitDto };

const props = withDefaults(
  defineProps<{
    ws: string;
    open: boolean;
    actions?: LocalAction[];
  }>(),
  {
    actions: () => [],
  },
);

const emit = defineEmits<{
  (e: 'select', payload: PaletteSelection): void;
  (e: 'close'): void;
}>();

type Entry = { kind: 'action'; action: LocalAction } | { kind: 'hit'; hit: SearchHitDto };

const { store, onQueryInput } = useSearch(props.ws);

const inputEl = ref<HTMLInputElement | null>(null);
const activeIndex = ref(0);
const queryText = ref('');

const matchedActions = computed(() => filterLocalActions(props.actions, queryText.value));

const entries = computed<Entry[]>(() => [
  ...matchedActions.value.map((action): Entry => ({ kind: 'action', action })),
  ...store.results.map((hit): Entry => ({ kind: 'hit', hit })),
]);

const hasQuery = computed(() => queryText.value.trim() !== '');
const showEmpty = computed(() => hasQuery.value && entries.value.length === 0 && !store.loading);

function onInput(event: Event): void {
  const value = (event.target as HTMLInputElement).value;
  queryText.value = value;
  activeIndex.value = 0;
  onQueryInput(value);
}

function clampActive(): void {
  const max = entries.value.length - 1;
  if (activeIndex.value > max) activeIndex.value = Math.max(0, max);
  if (activeIndex.value < 0) activeIndex.value = 0;
}

function move(delta: number): void {
  const count = entries.value.length;
  if (count === 0) return;
  activeIndex.value = (activeIndex.value + delta + count) % count;
}

function selectEntry(entry: Entry | undefined): void {
  if (entry === undefined) return;
  if (entry.kind === 'action') {
    emit('select', { type: 'action', action: entry.action });
  } else {
    emit('select', { type: 'hit', hit: entry.hit });
  }
}

function onKeydown(event: KeyboardEvent): void {
  switch (event.key) {
    case 'ArrowDown':
      event.preventDefault();
      move(1);
      break;
    case 'ArrowUp':
      event.preventDefault();
      move(-1);
      break;
    case 'Enter':
      event.preventDefault();
      selectEntry(entries.value[activeIndex.value]);
      break;
    case 'Escape':
      event.preventDefault();
      emit('close');
      break;
  }
}

function snippetHtml(hit: SearchHitDto): string {
  return hit.snippet ? sanitizeSnippet(hit.snippet) : '';
}

watch(entries, clampActive);

watch(
  () => props.open,
  (open) => {
    if (open) {
      void nextTick(() => inputEl.value?.focus());
    }
  },
  { immediate: true },
);
</script>

<template>
  <div
    v-if="open"
    class="fixed inset-0 flex justify-center"
    :style="{ background: 'var(--c-overlay)', paddingTop: '12vh', zIndex: 50 }"
    data-kind="command-palette"
    @click.self="emit('close')"
  >
    <div
      class="flex flex-col w-full"
      :style="{
        maxWidth: '600px',
        maxHeight: '60vh',
        background: 'var(--c-panel)',
        border: '1px solid var(--c-border)',
        borderRadius: 'var(--r-lg)',
        boxShadow: 'var(--shadow-lg)',
        overflow: 'hidden',
      }"
    >
      <div
        class="flex items-center gap-2"
        :style="{ padding: '10px 12px', borderBottom: '1px solid var(--c-border)' }"
      >
        <Icon name="search" :size="15" :style="{ color: 'var(--c-muted)' }" />
        <input
          ref="inputEl"
          type="text"
          placeholder="Search documents, tasks, or jump to…"
          :value="queryText"
          :style="{
            flex: 1,
            background: 'transparent',
            border: 'none',
            outline: 'none',
            color: 'var(--c-foreground)',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-base)',
          }"
          @input="onInput"
          @keydown="onKeydown"
        >
        <Kbd>ESC</Kbd>
      </div>

      <div class="flex-1 overflow-y-auto" :style="{ minHeight: 0 }">
        <button
          v-for="(entry, i) in entries"
          :key="entry.kind === 'action' ? `a-${entry.action.id}` : `h-${entry.hit.id}`"
          type="button"
          class="atl-pal-row flex w-full items-center gap-3 text-left"
          :data-active="i === activeIndex ? 'true' : 'false'"
          :style="{
            padding: '9px 13px',
            cursor: 'pointer',
            background: i === activeIndex ? 'var(--c-selection)' : 'transparent',
            boxShadow: i === activeIndex ? 'inset 2px 0 0 var(--c-primary)' : 'none',
          }"
          @mouseenter="activeIndex = i"
          @click="selectEntry(entry)"
        >
          <template v-if="entry.kind === 'action'">
            <Icon
              :name="entry.action.kind === 'navigate' ? 'corner-down-right' : 'plus'"
              :size="15"
              :style="{ color: i === activeIndex ? 'var(--c-primary)' : 'var(--c-muted)' }"
            />
            <span :style="{ fontSize: 'var(--fs-base)', color: 'var(--c-foreground)', flex: 1 }">
              {{ entry.action.label }}
            </span>
          </template>

          <template v-else>
            <Icon
              :name="entry.hit.kind === 'task' ? 'square-check-big' : 'file-text'"
              :size="15"
              :style="{ color: i === activeIndex ? 'var(--c-primary)' : 'var(--c-muted)', flex: '0 0 auto' }"
            />
            <span class="flex-1 min-w-0">
              <span class="block truncate" :style="{ fontSize: 'var(--fs-base)', color: 'var(--c-foreground)' }">
                <span
                  v-if="entry.hit.kind === 'task' && entry.hit.readable_id"
                  :style="{ fontFamily: 'var(--font-mono)', color: 'var(--c-muted)', marginRight: '6px' }"
                >{{ entry.hit.readable_id }}</span>{{ entry.hit.title }}
              </span>
              <!-- eslint-disable-next-line vue/no-v-html -- sanitizeSnippet allows only <mark> (REQ-W25) -->
              <span
                v-if="entry.hit.snippet"
                class="atl-pal-snip block truncate"
                :style="{ fontSize: 'var(--fs-xs)', color: 'var(--c-muted)' }"
                v-html="snippetHtml(entry.hit)"
              />
            </span>
            <Chip tone="neutral">{{ entry.hit.kind === 'task' ? 'TASK' : 'NOTE' }}</Chip>
          </template>
        </button>

        <div
          v-if="showEmpty"
          :style="{ padding: '24px 13px', textAlign: 'center', fontSize: 'var(--fs-sm)', color: 'var(--c-muted)' }"
        >
          No results for "{{ queryText }}"
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.atl-pal-snip :deep(mark) {
  background: rgba(255, 180, 84, 0.25);
  color: var(--c-foreground);
  border-radius: 2px;
  padding: 0 2px;
}
</style>
