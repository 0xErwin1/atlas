<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useRouter } from 'vue-router';
import ResultRow from '@/components/search/ResultRow.vue';
import SearchPreview from '@/components/search/SearchPreview.vue';
import EditorToolbar from '@/components/shell/EditorToolbar.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import Btn from '@/components/ui/Btn.vue';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import { useSearch } from '@/composables/useSearch';
import { type SearchHitDto, type SearchSort, type SearchType, useSearchStore } from '@/stores/search';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';
import AppShell from '@/views/AppShell.vue';
import SearchSidebar from '@/views/SearchSidebar.vue';

const router = useRouter();
const workspace = useWorkspaceStore();
const ui = useUiStore();
const { isMobile } = useBreakpoint();

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const { store, onQueryInput, loadMore } = useSearch(ws.value);
const searchStore = useSearchStore();

const SCOPE_CHIPS: Array<{ value: SearchType; label: string }> = [
  { value: 'all', label: 'All' },
  { value: 'note', label: 'Notes' },
  { value: 'task', label: 'Tasks' },
];

function onScope(value: SearchType): void {
  searchStore.setType(value);
  void store.runSearch(ws.value);
}

const activeIndex = ref(0);

const activeHit = computed<SearchHitDto | null>(() => store.results[activeIndex.value] ?? null);

const SORT_OPTIONS: Array<{ value: SearchSort; label: string }> = [
  { value: 'relevance', label: 'Relevance' },
  { value: 'updated', label: 'Recently updated' },
];

const sortLabel = computed(
  () => SORT_OPTIONS.find((o) => o.value === searchStore.sort)?.label ?? 'Relevance',
);

function onSort(value: SearchSort): void {
  searchStore.setSort(value);
  void store.runSearch(ws.value);
}

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

// Populate the sidebar's Project facet (the search view itself loads no projects).
onMounted(() => {
  if (workspace.projects.length === 0 && ws.value !== '') {
    void workspace.loadProjects(ws.value);
  }
});

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
  <AppShell sidebar-title="Search" sidebar-icon="search" :mobile-detail="true">
    <template #sidebar>
      <SearchSidebar :query="store.query" @input="onInput" @clear="clearSearch" @rerun="rerun" />
    </template>

    <template #sidebar-footer>
      <button
        type="button"
        class="atl-gbtn"
        style="width: 100%; justify-content: flex-start; height: 26px; gap: 7px; color: var(--c-foreground);"
        @click="ui.showBanner('Saved searches are coming soon', 'info')"
      >
        <Icon name="star" :size="14" />
        Save this search
      </button>
    </template>

    <div
      v-if="isMobile"
      class="flex flex-col"
      style="padding: 10px 12px; gap: 10px; border-bottom: 1px solid var(--c-border);"
    >
      <div
        class="flex items-center"
        style="gap: 8px; height: 36px; padding: 0 10px; background: var(--c-input); border: 1px solid var(--c-border); border-radius: var(--r-md);"
      >
        <Icon name="search" :size="15" :style="{ color: 'var(--c-muted)' }" />
        <input
          type="text"
          placeholder="Search documents and tasks…"
          autocomplete="off"
          :value="store.query"
          class="flex-1 min-w-0"
          style="height: 100%; border: none; outline: none; background: transparent; color: var(--c-foreground); font-size: var(--fs-base);"
          @input="onInput(($event.target as HTMLInputElement).value)"
        >
        <button
          v-if="store.query"
          type="button"
          aria-label="Clear search"
          class="inline-flex items-center cursor-pointer"
          style="border: none; background: transparent; color: var(--c-muted);"
          @click="clearSearch"
        >
          <Icon name="x" :size="14" />
        </button>
      </div>

      <div class="flex" style="gap: 6px;">
        <button
          v-for="chip in SCOPE_CHIPS"
          :key="chip.value"
          type="button"
          :aria-pressed="searchStore.type === chip.value"
          :style="`
            height: 28px;
            padding: 0 12px;
            border-radius: 9999px;
            cursor: pointer;
            font-size: var(--fs-sm);
            font-weight: var(--fw-medium);
            border: 1px solid ${searchStore.type === chip.value ? 'var(--c-primary)' : 'var(--c-border)'};
            background: ${searchStore.type === chip.value ? 'var(--c-selection)' : 'transparent'};
            color: ${searchStore.type === chip.value ? 'var(--c-primary)' : 'var(--c-muted)'};
          `"
          @click="onScope(chip.value)"
        >
          {{ chip.label }}
        </button>
      </div>
    </div>

    <EditorToolbar v-else :breadcrumbs="[]" :dirty="false">
      <span
        :style="{ fontSize: 'var(--fs-base)', fontWeight: 'var(--fw-bold)', color: 'var(--c-foreground)' }"
      >
        {{ resultCountLabel }}
      </span>
      <span
        v-if="store.query"
        :style="{ fontSize: 'var(--fs-sm)', color: 'var(--c-muted)', fontFamily: 'var(--font-mono)' }"
      >
        for "{{ store.query }}"
      </span>

      <div style="flex: 1;" />

      <Popover placement="bottom-end">
        <template #trigger="{ open, toggle }">
          <button
            type="button"
            class="inline-flex items-center cursor-pointer select-none"
            :style="{
              gap: '6px',
              fontSize: 'var(--fs-sm)',
              color: 'var(--c-muted)',
              border: '1px solid var(--c-border)',
              borderRadius: 'var(--r-sm)',
              padding: '4px 9px',
              background: 'transparent',
            }"
            @click="toggle"
          >
            {{ sortLabel }}
            <Icon
              name="chevron-down"
              :size="12"
              :style="{
                flex: '0 0 auto',
                transform: open ? 'rotate(180deg)' : 'none',
                transition: 'transform 0.1s',
              }"
            />
          </button>
        </template>

        <template #default="{ close }">
          <ul role="listbox" style="list-style: none; padding: 2px 0; min-width: 100%;">
            <li
              v-for="opt in SORT_OPTIONS"
              :key="opt.value"
              role="option"
              :aria-selected="opt.value === searchStore.sort"
              class="flex items-center px-3 cursor-pointer"
              :style="`
                height: var(--h-compact);
                white-space: nowrap;
                font-size: var(--fs-sm);
                ${opt.value === searchStore.sort ? 'background-color: var(--c-selection); color: var(--c-foreground);' : 'color: var(--c-foreground);'}
              `"
              @click="onSort(opt.value), close()"
            >
              {{ opt.label }}
            </li>
          </ul>
        </template>
      </Popover>
      <button
        type="button"
        class="atl-gbtn"
        title="Command palette ⌘K"
        aria-label="Command palette"
        @click="ui.openPalette()"
      >
        <Icon name="command" :size="14" />
      </button>
    </EditorToolbar>

    <div class="flex flex-1 min-h-0">
      <div
        class="flex-1 overflow-y-auto outline-none min-w-0"
        tabindex="0"
        :style="{ background: 'var(--c-background)' }"
        @keydown="onListKeydown"
      >
        <ErrorState
          v-if="store.error"
          title="Couldn’t search"
          :hint="store.error"
          @retry="onQueryInput(store.query)"
        />
        <LoadingState
          v-else-if="store.loading && store.results.length === 0"
          label="Searching…"
        />

        <template v-else>
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

          <EmptyState
            v-if="store.query && store.results.length === 0 && !store.loading"
            :title="`No results for “${store.query}”`"
            hint="Try a different term, or broaden the type filter"
            icon="search-x"
          />

          <EmptyState
            v-else-if="!store.query"
            title="Search documents and tasks"
            hint="Search across the workspace by title, content, or @handle"
            icon="search"
          />
        </template>
      </div>

      <SearchPreview v-if="!isMobile && activeHit" :hit="activeHit" @open="navigateToHit" />
    </div>
  </AppShell>
</template>
