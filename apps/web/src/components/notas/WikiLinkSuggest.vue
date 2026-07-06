<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import Icon from '@/components/ui/Icon.vue';

type SearchHit = components['schemas']['SearchHitDto'];

const props = defineProps<{
  ws: string;
  /** Active `[[` query, or null when no trigger is open. */
  query: string | null;
}>();

const emit = defineEmits<{
  /**
   * A note was chosen. An existing hit carries its stable id (the link binds to
   * it); a free-typed "Create" carries a null id (resolved by slug, pending).
   */
  select: [ref: { id: string | null; title: string }];
}>();

const hits = ref<SearchHit[]>([]);
const degraded = ref(false);
const activeIndex = ref(0);

const open = computed(() => props.query !== null);

/**
 * Fetches note candidates from `search?type=note` (REQ-W16). A network/API
 * error degrades gracefully: the dropdown still offers free-typed creation so
 * the user is never blocked.
 */
async function fetchNotes(q: string): Promise<void> {
  const { data, error } = await wrappedClient.GET('/api/workspaces/{ws}/search', {
    params: { path: { ws: props.ws }, query: { q: q.length > 0 ? q : '*', type: 'note', limit: 8 } },
  });

  if (error !== undefined || data === undefined) {
    degraded.value = true;
    hits.value = [];
    return;
  }

  degraded.value = false;
  hits.value = data.items;
}

watch(
  () => props.query,
  (q) => {
    activeIndex.value = 0;
    if (q === null) {
      hits.value = [];
      return;
    }
    void fetchNotes(q);
  },
  { immediate: true },
);

const createLabel = computed(() => (props.query ?? '').trim());
const canCreate = computed(() => createLabel.value.length > 0);

const itemCount = computed(() => hits.value.length + (canCreate.value ? 1 : 0));

function choose(index: number): void {
  if (index < hits.value.length) {
    const hit = hits.value[index];
    if (hit !== undefined) emit('select', { id: hit.id, title: hit.title });
    return;
  }
  if (canCreate.value) emit('select', { id: null, title: createLabel.value });
}

function moveDown(): void {
  if (itemCount.value === 0) return;
  activeIndex.value = (activeIndex.value + 1) % itemCount.value;
}

function moveUp(): void {
  if (itemCount.value === 0) return;
  activeIndex.value = (activeIndex.value - 1 + itemCount.value) % itemCount.value;
}

function confirmActive(): void {
  choose(activeIndex.value);
}

defineExpose({ open, moveDown, moveUp, confirmActive });
</script>

<template>
  <div
    v-if="open"
    role="listbox"
    aria-label="Link to note"
    style="
      position: absolute;
      z-index: 30;
      width: 250px;
      background: var(--c-raised);
      border: 1px solid var(--c-border);
      border-radius: var(--r-md);
      box-shadow: var(--shadow-lg);
      padding: 3px 0;
    "
  >
    <div
      style="
        padding: 3px 8px 4px;
        font-size: 10px;
        font-weight: var(--fw-semibold);
        color: var(--c-muted);
        text-transform: uppercase;
        letter-spacing: 0.06em;
      "
    >
      Link to note
    </div>

    <button
      v-for="(hit, i) in hits"
      :key="hit.id"
      type="button"
      role="option"
      :aria-selected="activeIndex === i"
      class="flex items-center gap-2 w-full text-left"
      :style="`
        height: 26px;
        padding: 0 8px;
        border: none;
        cursor: pointer;
        font-size: var(--fs-sm);
        color: var(--c-foreground);
        background: ${activeIndex === i ? 'var(--c-list-active)' : 'transparent'};
      `"
      @mouseenter="activeIndex = i"
      @mousedown.prevent="choose(i)"
    >
      <Icon name="file" :size="14" />
      <span class="truncate">{{ hit.title }}</span>
    </button>

    <button
      v-if="canCreate"
      type="button"
      role="option"
      :aria-selected="activeIndex === hits.length"
      class="flex items-center gap-2 w-full text-left"
      :style="`
        height: 26px;
        padding: 0 8px;
        border: none;
        cursor: pointer;
        font-size: var(--fs-sm);
        color: var(--c-foreground);
        background: ${activeIndex === hits.length ? 'var(--c-list-active)' : 'transparent'};
      `"
      @mouseenter="activeIndex = hits.length"
      @mousedown.prevent="choose(hits.length)"
    >
      <Icon name="plus" :size="14" />
      <span class="flex-1">
        Create
        <span style="font-family: var(--font-mono); color: var(--c-primary);">"{{ createLabel }}"</span>
      </span>
      <span
        style="
          font-size: 10px;
          font-weight: var(--fw-bold);
          letter-spacing: 0.04em;
          text-transform: uppercase;
          color: var(--c-muted);
          background: var(--c-panel);
          border-radius: var(--r-sm);
          padding: 1px 5px;
        "
      >
        new
      </span>
    </button>

    <div
      v-if="degraded"
      style="padding: 4px 8px; font-size: var(--fs-xs); color: var(--c-warning);"
    >
      Search unavailable — type a title and press Enter.
    </div>
  </div>
</template>
