<script setup lang="ts">
import { computed } from 'vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import GhTag from '@/components/ui/GhTag.vue';
import Icon from '@/components/ui/Icon.vue';
import Row from '@/components/ui/Row.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import { swatchById } from '@/lib/swatches';
import { useLabelColorsStore } from '@/stores/labelColors';
import type { SearchType } from '@/stores/search';
import { useSearchStore } from '@/stores/search';
import { useWorkspaceStore } from '@/stores/workspace';

const props = defineProps<{
  query: string;
}>();

const emit = defineEmits<{
  (e: 'input', value: string): void;
  (e: 'clear'): void;
  (e: 'rerun'): void;
}>();

const store = useSearchStore();
const workspace = useWorkspaceStore();
const labelColors = useLabelColorsStore();

const typeOptions: DropdownOption[] = [
  { value: 'all', label: 'All types' },
  { value: 'note', label: 'Notes' },
  { value: 'task', label: 'Tasks' },
];

const STATUS_FILTERS = [
  { label: 'Open', token: 'status:open' },
  { label: 'In review', token: 'status:review' },
  { label: 'Done', token: 'status:done' },
];

const queryModel = computed(() => props.query);

function onType(value: string): void {
  store.setType(value as SearchType);
  emit('rerun');
}

// Facets are query tokens (the server parses `status:` / `project:` etc. out of
// `q`). Toggling a chip rewrites the query and re-runs; the chip's on-state is
// derived from whether its token is present.
function escapeRegex(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function hasToken(token: string): boolean {
  return new RegExp(`(^|\\s)${escapeRegex(token)}(\\s|$)`).test(store.query);
}

function toggleToken(token: string): void {
  let q = store.query;
  if (hasToken(token)) {
    q = q
      .replace(new RegExp(`(^|\\s)${escapeRegex(token)}(?=\\s|$)`), ' ')
      .replace(/\s+/g, ' ')
      .trim();
  } else {
    q = `${q.trim()} ${token}`.trim();
  }
  store.setQuery(q);
  emit('rerun');
}

function projectToken(slug: string): string {
  return `project:${slug}`;
}

function tagToken(name: string): string {
  return `tag:${name}`;
}

function tagDotColor(name: string): string {
  return swatchById(labelColors.colorFor(`tag:${name.toLowerCase()}`)).fg;
}

function pickRecent(value: string): void {
  store.setQuery(value);
  emit('rerun');
}
</script>

<template>
  <div>
    <div :style="{ padding: '8px 10px' }">
      <div
        class="flex items-center"
        :style="{
          height: '28px',
          gap: '7px',
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
          class="atl-x inline-flex items-center cursor-pointer"
          :style="{ color: 'var(--c-muted)', border: 'none', background: 'transparent', borderRadius: 'var(--r-sm)' }"
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

    <SectionLabel>Status</SectionLabel>
    <button
      v-for="s in STATUS_FILTERS"
      :key="s.token"
      type="button"
      class="atl-row search-check"
      :class="{ on: hasToken(s.token) }"
      :aria-pressed="hasToken(s.token)"
      @click="toggleToken(s.token)"
    >
      <span class="search-check-box" :class="{ on: hasToken(s.token) }">
        <Icon v-if="hasToken(s.token)" name="check" :size="11" :stroke-width="2.6" />
      </span>
      {{ s.label }}
    </button>

    <template v-if="workspace.projects.length > 0">
      <SectionLabel>Project</SectionLabel>
      <Row
        v-for="p in workspace.projects"
        :key="p.slug"
        :label="p.name"
        icon="folder"
        :active="hasToken(projectToken(p.slug))"
        @click="toggleToken(projectToken(p.slug))"
      />
    </template>

    <template v-if="labelColors.tagNames.length > 0">
      <SectionLabel>Tags</SectionLabel>
      <div class="search-tags">
        <button
          v-for="t in labelColors.tagNames"
          :key="t"
          type="button"
          class="search-tag-btn"
          :aria-pressed="hasToken(tagToken(t))"
          @click="toggleToken(tagToken(t))"
        >
          <GhTag :color="tagDotColor(t)" :active="hasToken(tagToken(t))">{{ t }}</GhTag>
        </button>
      </div>
    </template>

    <template v-if="store.recents.length > 0">
      <SectionLabel>Recent</SectionLabel>
      <Row
        v-for="q in store.recents"
        :key="q"
        :label="q"
        icon="clock"
        @click="pickRecent(q)"
      />
    </template>
  </div>
</template>

<style scoped>
.search-check {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  height: 24px;
  padding: 0 10px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: var(--fs-sm);
  color: var(--c-muted);
  text-align: left;
}

.search-check.on {
  color: var(--c-foreground);
}

.search-check-box {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  flex: 0 0 auto;
  border: 1px solid var(--c-muted);
  border-radius: var(--r-sm);
}

.search-check-box.on {
  border-color: var(--c-primary);
  background: var(--c-primary);
  color: var(--c-background);
}

.search-tags {
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
  padding: 2px 10px 6px;
}

.search-tag-btn {
  border: none;
  background: transparent;
  padding: 0;
  cursor: pointer;
}
</style>
