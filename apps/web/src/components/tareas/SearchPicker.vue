<script setup lang="ts">
/**
 * Debounced workspace search restricted to one entity type, rendering the hits as
 * a pick list. Emits the chosen hit and clears itself; the host decides what the
 * pick means (add a reference, attach a sub-task, …).
 */
import { onMounted, ref, watch } from 'vue';
import { wrappedClient } from '@/api/wrapper';
import Icon from '@/components/ui/Icon.vue';
import type { SearchHitDto } from '@/stores/search';

const props = withDefaults(
  defineProps<{
    ws: string;
    /** Entity type to search; the server rejects a mismatched target. */
    type: 'note' | 'task';
    placeholder: string;
    /** Readable ID to hide from the results, so a task cannot pick itself. */
    excludeReadableId?: string;
    /** Dialog variant: larger type and taller result list. */
    large?: boolean;
    /** Focus the input on mount. */
    autofocus?: boolean;
  }>(),
  {
    large: false,
    autofocus: false,
  },
);

const emit = defineEmits<{ pick: [hit: SearchHitDto] }>();

const query = ref('');
const results = ref<SearchHitDto[]>([]);
const searching = ref(false);
const inputRef = ref<HTMLInputElement | null>(null);

onMounted(() => {
  if (props.autofocus) inputRef.value?.focus();
});

let debounce: ReturnType<typeof setTimeout> | null = null;

watch([query, () => props.type], () => {
  if (debounce !== null) clearTimeout(debounce);

  const term = query.value.trim();
  if (term === '') {
    results.value = [];
    return;
  }

  debounce = setTimeout(async () => {
    searching.value = true;
    try {
      const { data } = await wrappedClient.GET('/api/workspaces/{ws}/search', {
        params: {
          path: { ws: props.ws },
          query: { q: term, type: props.type, sort: 'relevance', prefix: true },
        },
      });
      const items = data?.items ?? [];
      results.value =
        props.excludeReadableId != null
          ? items.filter((hit) => hit.readable_id !== props.excludeReadableId)
          : items;
    } catch {
      results.value = [];
    } finally {
      searching.value = false;
    }
  }, 220);
});

function pick(hit: SearchHitDto): void {
  if (hit.readable_id != null && hit.readable_id === props.excludeReadableId) return;

  emit('pick', hit);

  query.value = '';
  results.value = [];
}

defineExpose({ focus: (): void => inputRef.value?.focus() });
</script>

<template>
  <div class="atl-refadd-row">
    <!-- Hosts that pair the search with a control (e.g. a reference-kind
         dropdown) render it here so it shares the input's row. -->
    <slot name="leading" />

    <div class="atl-refadd-search" :class="{ lg: large }">
      <Icon name="search" :size="large ? 17 : 13" style="color: var(--c-muted); flex: 0 0 auto;" />
      <input
        ref="inputRef"
        v-model="query"
        type="text"
        :placeholder="placeholder"
        class="atl-refadd-input"
        :class="{ lg: large }"
      />
    </div>
  </div>

  <div v-if="results.length > 0" class="atl-refadd-results" :class="{ lg: large }">
    <button
      v-for="hit in results"
      :key="hit.id"
      type="button"
      class="atl-refadd-result"
      :class="{ lg: large }"
      @click="pick(hit)"
    >
      <Icon
        :name="hit.kind === 'task' ? 'square-kanban' : 'file-text'"
        :size="large ? 15 : 13"
        style="color: var(--c-muted); flex: 0 0 auto;"
      />
      <span class="atl-refadd-title" :class="{ lg: large }" :title="hit.title">{{ hit.title }}</span>
      <span v-if="hit.readable_id" class="atl-refadd-id">{{ hit.readable_id }}</span>
    </button>
  </div>
  <div v-else-if="query.trim() !== '' && !searching" class="atl-refadd-empty" :class="{ lg: large }">
    No matches.
  </div>
</template>

<style scoped>
.atl-refadd-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.atl-refadd-search {
  display: flex;
  align-items: center;
  gap: 7px;
  flex: 1;
  min-width: 0;
  height: var(--h-input);
  padding: 0 10px;
  background: var(--c-input);
  border: 1px solid var(--c-border);
}

.atl-refadd-input {
  flex: 1;
  min-width: 0;
  background: transparent;
  border: none;
  outline: none;
  color: var(--c-foreground);
  font-size: var(--fs-sm);
}

.atl-refadd-input::placeholder {
  color: var(--c-muted);
}

.atl-refadd-results {
  margin-top: 6px;
  border: 1px solid var(--c-border);
  background: var(--c-panel);
  overflow: hidden;
  max-height: 220px;
  overflow-y: auto;
}

.atl-refadd-result {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 7px 10px;
  border: none;
  background: transparent;
  cursor: pointer;
  text-align: left;
}

.atl-refadd-result:hover {
  background: var(--c-raised);
}

.atl-refadd-title {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-sm);
  color: var(--c-foreground);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-refadd-id {
  flex: 0 0 auto;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-refadd-empty {
  font-size: var(--fs-xs);
  color: var(--c-muted);
  padding: 6px 2px;
}

/* Large variant — used inside the Link-or-add-dependency dialog so the search
   reads at the same level as the app's command palette. */
.atl-refadd-search.lg {
  height: 42px;
  padding: 0 13px;
  gap: 9px;
}

.atl-refadd-input.lg {
  font-size: var(--fs-lg);
}

.atl-refadd-results.lg {
  margin-top: 10px;
  max-height: 46vh;
}

.atl-refadd-result.lg {
  padding: 10px 12px;
}

.atl-refadd-title.lg {
  font-size: var(--fs-base);
}

.atl-refadd-empty.lg {
  font-size: var(--fs-sm);
  padding: 12px 4px;
}
</style>
