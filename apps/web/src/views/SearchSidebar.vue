<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, type WritableComputedRef } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import MultiSelect, { type MultiSelectOption } from '@/components/ui/MultiSelect.vue';
import Row from '@/components/ui/Row.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import { swatchById } from '@/lib/swatches';
import { useLabelColorsStore } from '@/stores/labelColors';
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

const TYPE_OPTIONS: MultiSelectOption[] = [
  { value: 'note', label: 'Notes', icon: 'notes' },
  { value: 'task', label: 'Tasks', icon: 'square-kanban' },
  { value: 'doc', label: 'Docs', icon: 'file-text', disabled: true },
  { value: 'comment', label: 'Comments', icon: 'message-square', disabled: true },
];

const SUPPORTED_TYPES = ['note', 'task'] as const;
type SupportedType = (typeof SUPPORTED_TYPES)[number];

const typeModel = computed<string[]>({
  get: () => {
    const match = store.query.match(/(?:^|\s)type:(\S+)(?:\s|$)/);
    if (!match || match[1] === undefined) return [];
    return match[1]
      .split(',')
      .filter((v): v is SupportedType => SUPPORTED_TYPES.includes(v as SupportedType));
  },
  set: (values) => {
    const supported = values.filter((v): v is SupportedType => SUPPORTED_TYPES.includes(v as SupportedType));

    const stripped = store.query
      .replace(/(?:^|\s)type:\S+/g, ' ')
      .replace(/\s+/g, ' ')
      .trim();

    if (supported.length === 0) {
      store.setQuery(stripped);
    } else {
      const token = `type:${supported.join(',')}`;
      store.setQuery(stripped === '' ? token : `${stripped} ${token}`);
    }

    emit('rerun');
  },
});

const STATUS_OPTIONS: MultiSelectOption[] = [
  { value: 'open', label: 'Open', dot: 'var(--c-info)' },
  { value: 'review', label: 'In review', dot: 'var(--c-primary)' },
  { value: 'done', label: 'Done', dot: 'var(--c-success)' },
  { value: 'blocked', label: 'Blocked', dot: 'var(--c-danger)' },
];

const queryModel = computed(() => props.query);

// Facets are query tokens (the server parses `status:` / `project:` etc. out of
// `q`). Toggling a chip rewrites the query and re-runs; the chip's on-state is
// derived from whether its token is present.
function escapeRegex(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function hasToken(token: string): boolean {
  return new RegExp(`(^|\\s)${escapeRegex(token)}(\\s|$)`).test(store.query);
}

function addToken(token: string): void {
  if (hasToken(token)) return;
  store.setQuery(`${store.query.trim()} ${token}`.trim());
  emit('rerun');
}

function removeToken(token: string): void {
  if (!hasToken(token)) return;
  const q = store.query
    .replace(new RegExp(`(^|\\s)${escapeRegex(token)}(?=\\s|$)`), ' ')
    .replace(/\s+/g, ' ')
    .trim();
  store.setQuery(q);
  emit('rerun');
}

function toggleToken(token: string): void {
  if (hasToken(token)) removeToken(token);
  else addToken(token);
}

// Bridges a MultiSelect (`string[]` of selected values) to the facet tokens that
// live inside `q`. The model reflects which tokens are present and writing it
// diffs each value against the current set, preserving the existing add/remove
// query behavior underneath. `values` is a getter so dynamic facets (tags) stay
// reactive.
function tokenModel(
  values: () => string[],
  tokenFor: (value: string) => string,
): WritableComputedRef<string[]> {
  return computed<string[]>({
    get: () => values().filter((v) => hasToken(tokenFor(v))),
    set: (next) => {
      for (const v of values()) {
        const token = tokenFor(v);
        const shouldBeOn = next.includes(v);
        if (shouldBeOn && !hasToken(token)) addToken(token);
        else if (!shouldBeOn && hasToken(token)) removeToken(token);
      }
    },
  });
}

const statusModel = tokenModel(
  () => STATUS_OPTIONS.map((o) => o.value),
  (value) => `status:${value}`,
);

const tagModel = tokenModel(
  () => labelColors.tagNames,
  (name) => tagToken(name),
);

const tagOptions = computed<MultiSelectOption[]>(() =>
  labelColors.tagNames.map((name) => ({
    value: name,
    label: name,
    dot: tagDotColor(name),
  })),
);

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

// The Search rail is 256px in the design, narrower than the shared 264px shell
// default. The width lives on the ContextSidebar `aside`, which this view does
// not own, so it is overridden here for the Search view only — the enclosing
// aside is found at mount and restored on unmount, leaving other views' shells
// untouched.
const SEARCH_SIDEBAR_WIDTH = '256px';
const rootEl = ref<HTMLElement | null>(null);
let restoreSidebarWidth: (() => void) | null = null;

onMounted(() => {
  const aside = rootEl.value?.closest('aside');
  if (!aside) return;

  const previous = {
    width: aside.style.width,
    flexBasis: aside.style.flexBasis,
    minWidth: aside.style.minWidth,
  };

  aside.style.width = SEARCH_SIDEBAR_WIDTH;
  aside.style.flexBasis = SEARCH_SIDEBAR_WIDTH;
  aside.style.minWidth = SEARCH_SIDEBAR_WIDTH;

  restoreSidebarWidth = () => {
    aside.style.width = previous.width;
    aside.style.flexBasis = previous.flexBasis;
    aside.style.minWidth = previous.minWidth;
  };
});

onBeforeUnmount(() => {
  restoreSidebarWidth?.();
});
</script>

<template>
  <div ref="rootEl">
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

    <SectionLabel>Type</SectionLabel>
    <div :style="{ padding: '0 10px 4px' }">
      <MultiSelect v-model="typeModel" :options="TYPE_OPTIONS" placeholder="Any type" />
    </div>

    <SectionLabel>Status</SectionLabel>
    <div :style="{ padding: '0 10px 4px' }">
      <MultiSelect v-model="statusModel" :options="STATUS_OPTIONS" placeholder="Any status" />
    </div>

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
      <div :style="{ padding: '0 10px 6px' }">
        <MultiSelect v-model="tagModel" :options="tagOptions" icon="tag" placeholder="Any tag" />
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
