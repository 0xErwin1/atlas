<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, type WritableComputedRef } from 'vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import MultiSelect, { type MultiSelectOption } from '@/components/ui/MultiSelect.vue';
import Row from '@/components/ui/Row.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { useInlineEdit } from '@/composables/useInlineEdit';
import { swatchById } from '@/lib/swatches';
import { useLabelColorsStore } from '@/stores/labelColors';
import { type SavedSearchDto, useSavedSearchesStore } from '@/stores/savedSearches';
import { useSearchStore } from '@/stores/search';
import { useUiStore } from '@/stores/ui';
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
const savedSearches = useSavedSearchesStore();
const ui = useUiStore();

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

// ── Saved searches: apply / rename (inline) / delete (context menu) ──────
const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const { open: menuOpen, x: menuX, y: menuY, openAt, close: closeMenu } = useContextMenu();
const menuTarget = ref<SavedSearchDto | null>(null);

const {
  active: editActive,
  value: editValue,
  inputRef,
  start: startEdit,
  commit: commitEdit,
  onKeydown: onEditKeydown,
} = useInlineEdit<{ id: string }>(async (name, ctx) => {
  if (ws.value === '') return;
  const ok = await savedSearches.rename(ws.value, ctx.id, name);
  if (!ok && savedSearches.error) ui.showBanner(savedSearches.error, 'error');
});

function applySaved(ss: SavedSearchDto): void {
  store.setQuery(ss.query);
  emit('rerun');
}

async function removeSaved(ss: SavedSearchDto): Promise<void> {
  if (ws.value === '') return;
  const ok = await savedSearches.remove(ws.value, ss.id);
  if (!ok && savedSearches.error) ui.showBanner(savedSearches.error, 'error');
}

const savedMenuItems = computed<MenuItem[]>(() => {
  const t = menuTarget.value;
  if (t === null) return [];
  return [
    { header: true, label: t.name },
    { label: 'Apply', icon: 'corner-down-left', action: () => applySaved(t) },
    { sep: true },
    { label: 'Rename', icon: 'pencil', action: () => startEdit({ id: t.id }, t.name, true) },
    { label: 'Delete', icon: 'trash-2', danger: true, action: () => void removeSaved(t) },
  ];
});

function openSavedMenu(event: MouseEvent, ss: SavedSearchDto): void {
  menuTarget.value = ss;
  openAt(event);
}

onMounted(() => {
  if (ws.value !== '') void savedSearches.load(ws.value);
});

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

    <template v-if="savedSearches.items.length > 0">
      <SectionLabel>Saved</SectionLabel>
      <template v-for="ss in savedSearches.items" :key="ss.id">
        <div
          v-if="editActive?.id === ss.id"
          style="display: flex; align-items: center; gap: 6px; padding: 3px 8px;"
        >
          <Icon name="star" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
          <input
            ref="inputRef"
            v-model="editValue"
            type="text"
            placeholder="Search name…"
            class="saved-inline-input"
            @keydown="onEditKeydown"
            @blur="commitEdit"
          />
        </div>
        <Row
          v-else
          :label="ss.name"
          icon="star"
          menu
          @click="applySaved(ss)"
          @menu="(event: MouseEvent) => openSavedMenu(event, ss)"
          @contextmenu.prevent.stop="(event: MouseEvent) => openSavedMenu(event, ss)"
        />
      </template>
    </template>

    <ContextMenu
      :open="menuOpen"
      :x="menuX"
      :y="menuY"
      :items="savedMenuItems"
      @close="closeMenu"
    />
  </div>
</template>

<style scoped>
.saved-inline-input {
  flex: 1;
  height: 28px;
  padding: 0 6px;
  background: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  font-size: var(--fs-sm);
  font-family: var(--font-mono);
  color: var(--c-foreground);
  outline: none;
}
</style>
