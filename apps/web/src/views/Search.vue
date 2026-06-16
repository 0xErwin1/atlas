<script setup lang="ts">
import { computed, ref } from 'vue';
import { useRouter } from 'vue-router';
import ResultRow from '@/components/search/ResultRow.vue';
import EditorToolbar from '@/components/shell/EditorToolbar.vue';
import Btn from '@/components/ui/Btn.vue';
import { useSearch } from '@/composables/useSearch';
import type { SearchHitDto } from '@/stores/search';
import { useWorkspaceStore } from '@/stores/workspace';
import AppShell from '@/views/AppShell.vue';
import SearchSidebar from '@/views/SearchSidebar.vue';

const router = useRouter();
const workspace = useWorkspaceStore();

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const { store, onQueryInput, loadMore } = useSearch(ws.value);

const activeIndex = ref(0);

const resultCountLabel = computed(() => {
  const n = store.results.length;
  return `${n}${store.hasMore ? '+' : ''} result${n === 1 ? '' : 's'}`;
});

function navigateToHit(hit: SearchHitDto): void {
  if (hit.kind === 'task' && hit.readable_id) {
    void router.push({ name: 'task-detail', params: { readableId: hit.readable_id } });
    return;
  }
  void router.push({ name: 'notes', params: { slug: hit.id } });
}

function onInput(value: string): void {
  activeIndex.value = 0;
  onQueryInput(value);
}

function rerun(): void {
  void store.runSearch(ws.value);
}

function clearSearch(): void {
  store.clear();
}

function move(delta: number): void {
  const count = store.results.length;
  if (count === 0) return;
  activeIndex.value = (activeIndex.value + delta + count) % count;
}

function onListKeydown(event: KeyboardEvent): void {
  switch (event.key) {
    case 'ArrowDown':
      event.preventDefault();
      move(1);
      break;
    case 'ArrowUp':
      event.preventDefault();
      move(-1);
      break;
    case 'Enter': {
      event.preventDefault();
      const hit = store.results[activeIndex.value];
      if (hit) navigateToHit(hit);
      break;
    }
  }
}
</script>

<template>
  <AppShell>
    <template #sidebar>
      <SearchSidebar :query="store.query" @input="onInput" @clear="clearSearch" @rerun="rerun" />
    </template>

    <EditorToolbar :breadcrumbs="['Atlas', 'Search']" :dirty="false">
      <span
        :style="{ fontSize: 'var(--fs-base)', fontWeight: 'var(--fw-bold)', color: 'var(--c-foreground)', marginRight: '8px' }"
      >
        {{ resultCountLabel }}
      </span>
      <span
        v-if="store.query"
        :style="{ fontSize: 'var(--fs-sm)', color: 'var(--c-muted)', fontFamily: 'var(--font-mono)' }"
      >
        for "{{ store.query }}"
      </span>
    </EditorToolbar>

    <div
      class="flex-1 overflow-y-auto outline-none"
      tabindex="0"
      :style="{ background: 'var(--c-background)' }"
      @keydown="onListKeydown"
    >
      <p
        v-if="store.error"
        :style="{
          margin: '12px 16px',
          padding: '8px 12px',
          borderRadius: 'var(--r-md)',
          background: 'var(--c-banner-err-bg)',
          color: 'var(--c-banner-err-fg)',
          fontSize: 'var(--fs-sm)',
        }"
      >
        {{ store.error }}
      </p>

      <ResultRow
        v-for="(hit, i) in store.results"
        :key="hit.id"
        :hit="hit"
        :active="i === activeIndex"
        @click="navigateToHit(hit)"
        @mouseenter="activeIndex = i"
      />

      <div
        v-if="store.hasMore"
        :style="{ display: 'flex', justifyContent: 'center', padding: '12px' }"
      >
        <Btn variant="secondary" @click="loadMore">Load more</Btn>
      </div>

      <div
        v-if="store.query && store.results.length === 0 && !store.loading && !store.error"
        :style="{ padding: '48px 16px', textAlign: 'center', color: 'var(--c-muted)', fontSize: 'var(--fs-sm)' }"
      >
        No results for "{{ store.query }}".
      </div>

      <div
        v-if="!store.query"
        :style="{ padding: '48px 16px', textAlign: 'center', color: 'var(--c-muted)', fontSize: 'var(--fs-sm)' }"
      >
        Search documents and tasks across the workspace.
      </div>
    </div>
  </AppShell>
</template>
