<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { wrappedClient } from '@/api/wrapper';
import Icon from '@/components/ui/Icon.vue';
import { WIKILINK_KINDS, type WikilinkKind, type WikilinkRef } from '@/lib/wikilink';

/**
 * The resource whose attachments a `[[file:` query can address. Attachments have
 * no workspace-wide name, so they are only offered when the host says which
 * document or task the text being edited belongs to.
 */
export type AttachmentOwner = { kind: 'document'; slug: string } | { kind: 'task'; readableId: string };

const props = defineProps<{
  ws: string;
  /** Active `[[` query, or null when no trigger is open. */
  query: string | null;
  attachmentOwner?: AttachmentOwner | null;
}>();

const emit = defineEmits<{
  select: [ref: WikilinkRef];
}>();

interface Suggestion {
  key: string;
  label: string;
  icon: string;
  hint: string | null;
  ref: WikilinkRef;
}

const suggestions = ref<Suggestion[]>([]);
const degraded = ref(false);
const activeIndex = ref(0);

const open = computed(() => props.query !== null);

interface Scope {
  kind: WikilinkKind | null;
  term: string;
}

/**
 * Splits an active query into the kind it is scoped to and the term to match.
 * `note:runb` searches notes only; a bare `runb` searches notes and tasks
 * together, which is what an author usually wants before deciding.
 */
function scopeOf(query: string): Scope {
  const colon = query.indexOf(':');
  if (colon === -1) return { kind: null, term: query };

  const kind = query.slice(0, colon).toLowerCase();
  const known = (WIKILINK_KINDS as readonly string[]).includes(kind);

  return known ? { kind: kind as WikilinkKind, term: query.slice(colon + 1) } : { kind: null, term: query };
}

const scope = computed(() => scopeOf(props.query ?? ''));

/** Free-typed creation only makes sense for a note addressed by its title. */
const createLabel = computed(() =>
  scope.value.kind === null || scope.value.kind === 'note' ? scope.value.term.trim() : '',
);
const canCreate = computed(() => createLabel.value.length > 0);

const itemCount = computed(() => suggestions.value.length + (canCreate.value ? 1 : 0));

const listLabel = computed(() => {
  switch (scope.value.kind) {
    case 'task':
      return 'Link to task';
    case 'file':
      return 'Link to file';
    case 'note':
      return 'Link to note';
    default:
      return 'Link to note or task';
  }
});

async function fetchAttachments(term: string): Promise<Suggestion[]> {
  const owner = props.attachmentOwner ?? null;
  if (owner === null) return [];

  const { data, error } =
    owner.kind === 'document'
      ? await wrappedClient.GET('/api/workspaces/{ws}/documents/{slug}/attachments', {
          params: { path: { ws: props.ws, slug: owner.slug } },
        })
      : await wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/attachments', {
          params: { path: { ws: props.ws, readable_id: owner.readableId } },
        });

  if (error !== undefined || data === undefined) throw new Error('attachments unavailable');

  const needle = term.trim().toLowerCase();

  return data
    .filter((item) => needle.length === 0 || item.file_name.toLowerCase().includes(needle))
    .slice(0, 8)
    .map((item) => ({
      key: item.id,
      label: item.file_name,
      icon: 'paperclip',
      hint: null,
      ref: { target: { kind: 'file', fileName: item.file_name }, display: item.file_name },
    }));
}

async function fetchSearch(kind: WikilinkKind | null, term: string): Promise<Suggestion[]> {
  const type = kind === null ? 'note,task' : kind;

  const { data, error } = await wrappedClient.GET('/api/workspaces/{ws}/search', {
    params: {
      path: { ws: props.ws },
      query: { q: term.length > 0 ? term : '*', type, limit: 8 },
    },
  });

  if (error !== undefined || data === undefined) throw new Error('search unavailable');

  return data.items.flatMap((hit): Suggestion[] => {
    if (hit.kind === 'task') {
      const readableId = hit.readable_id ?? null;
      if (readableId === null) return [];

      return [
        {
          key: hit.id,
          label: hit.title,
          icon: 'square-check',
          hint: readableId,
          ref: { target: { kind: 'task', readableId }, display: hit.title },
        },
      ];
    }

    const slug = hit.document_slug ?? null;
    if (slug === null) return [];

    return [
      {
        key: hit.id,
        label: hit.title,
        icon: 'file',
        hint: null,
        ref: { target: { kind: 'note', slug }, display: hit.title },
      },
    ];
  });
}

/**
 * Loads candidates for the active query. A network/API error degrades
 * gracefully: the dropdown still offers free-typed creation so the user is
 * never blocked.
 */
async function load(): Promise<void> {
  const { kind, term } = scope.value;

  try {
    suggestions.value = kind === 'file' ? await fetchAttachments(term) : await fetchSearch(kind, term);
    degraded.value = false;
  } catch {
    degraded.value = true;
    suggestions.value = [];
  }
}

watch(
  () => props.query,
  (q) => {
    activeIndex.value = 0;
    if (q === null) {
      suggestions.value = [];
      return;
    }
    void load();
  },
  { immediate: true },
);

function choose(index: number): void {
  const suggestion = suggestions.value[index];
  if (suggestion !== undefined) {
    emit('select', suggestion.ref);
    return;
  }

  if (canCreate.value) {
    emit('select', { target: { kind: 'title' }, display: createLabel.value });
  }
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
    :aria-label="listLabel"
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
      {{ listLabel }}
    </div>

    <button
      v-for="(suggestion, i) in suggestions"
      :key="suggestion.key"
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
      <Icon :name="suggestion.icon" :size="14" />
      <span class="min-w-0 flex-1 truncate">{{ suggestion.label }}</span>
      <span
        v-if="suggestion.hint !== null"
        style="font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--c-muted);"
      >
        {{ suggestion.hint }}
      </span>
    </button>

    <button
      v-if="canCreate"
      type="button"
      role="option"
      :aria-selected="activeIndex === suggestions.length"
      class="flex items-center gap-2 w-full text-left"
      :style="`
        height: 26px;
        padding: 0 8px;
        border: none;
        cursor: pointer;
        font-size: var(--fs-sm);
        color: var(--c-foreground);
        background: ${activeIndex === suggestions.length ? 'var(--c-list-active)' : 'transparent'};
      `"
      @mouseenter="activeIndex = suggestions.length"
      @mousedown.prevent="choose(suggestions.length)"
    >
      <Icon name="plus" :size="14" />
      <span class="min-w-0 flex-1 truncate">
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
