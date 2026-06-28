<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import Icon from '@/components/ui/Icon.vue';
import type { SearchHitDto } from '@/stores/search';

type CreateReferenceRequest = components['schemas']['CreateReferenceRequest'];

const props = withDefaults(defineProps<{ ws: string; defaultKind?: string; large?: boolean }>(), {
  defaultKind: 'relates',
  large: false,
});

const emit = defineEmits<{ add: [body: CreateReferenceRequest] }>();

const KIND_OPTIONS: DropdownOption[] = [
  { value: 'relates', label: 'Relates to', icon: 'link' },
  { value: 'blocks', label: 'Blocks', icon: 'ban' },
  { value: 'parent', label: 'Parent', icon: 'git-branch' },
  { value: 'spec', label: 'Spec', icon: 'file-text' },
  { value: 'docs', label: 'Documentation', icon: 'book-text' },
];

const kind = ref(props.defaultKind);
const query = ref('');
const results = ref<SearchHitDto[]>([]);
const searching = ref(false);
const inputRef = ref<HTMLInputElement | null>(null);

// In the dialog (large) variant the search is the primary action, so focus it on
// mount the way the command palette does.
onMounted(() => {
  if (props.large) inputRef.value?.focus();
});

// The server requires a document target for `spec`/`docs` and a task target for
// the others, so the picker only searches the valid target type for the kind.
const targetType = computed<'note' | 'task'>(() =>
  kind.value === 'spec' || kind.value === 'docs' ? 'note' : 'task',
);
const placeholder = computed(() => (targetType.value === 'note' ? 'Link a note…' : 'Link a task…'));

let debounce: ReturnType<typeof setTimeout> | null = null;

watch([query, kind], () => {
  if (debounce !== null) clearTimeout(debounce);

  const term = query.value.trim();
  if (term === '') {
    results.value = [];
    return;
  }

  debounce = setTimeout(async () => {
    searching.value = true;
    try {
      const { data } = await wrappedClient.GET('/v1/workspaces/{ws}/search', {
        params: {
          path: { ws: props.ws },
          query: { q: term, type: targetType.value, sort: 'relevance', prefix: true },
        },
      });
      results.value = data?.items ?? [];
    } catch {
      results.value = [];
    } finally {
      searching.value = false;
    }
  }, 220);
});

function pick(hit: SearchHitDto): void {
  const body: CreateReferenceRequest =
    hit.kind === 'task'
      ? { kind: kind.value, target_task_readable_id: hit.readable_id ?? null }
      : { kind: kind.value, target_document_id: hit.id };

  emit('add', body);

  query.value = '';
  results.value = [];
}
</script>

<template>
  <div class="atl-refadd" :class="{ lg: large }">
    <div class="flex items-center" style="gap: 8px;">
      <Dropdown :options="KIND_OPTIONS" :model-value="kind" @change="(v) => (kind = v)" />
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
        <span class="atl-refadd-title" :class="{ lg: large }">{{ hit.title }}</span>
        <span v-if="hit.readable_id" class="atl-refadd-id">{{ hit.readable_id }}</span>
      </button>
    </div>
    <div
      v-else-if="query.trim() !== '' && !searching"
      class="atl-refadd-empty"
      :class="{ lg: large }"
    >
      No matches.
    </div>
  </div>
</template>

<style scoped>
.atl-refadd {
  position: relative;
  margin-top: 8px;
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
  border-radius: var(--r-md);
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
  border-radius: var(--r-md);
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
