<script setup lang="ts">
import { computed } from 'vue';
import type { components } from '@/api/types.d.ts';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import Icon from '@/components/ui/Icon.vue';

type Backlink = components['schemas']['Page_BacklinkDto']['items'][number];

type BacklinkRow = {
  id: string;
  label: string;
  detail: string;
  documentSlug: string | null;
  taskReadableId: string | null;
};

const props = defineProps<{
  backlinks: Backlink[];
  status: 'idle' | 'pending' | 'ready' | 'error';
  error: string | null;
}>();

const emit = defineEmits<{
  navigate: [slug: string];
  'navigate-task': [readableId: string];
  retry: [];
}>();

function backlinkRow(link: Backlink): BacklinkRow {
  const source = link.comment_source;

  if (source?.parent.type === 'task') {
    return {
      id: source.comment_id,
      label: source.parent.title,
      detail: link.display_title,
      documentSlug: null,
      taskReadableId: source.parent.readable_id,
    };
  }

  if (source?.parent.type === 'document') {
    const slug = source.parent.slug ?? null;
    return {
      id: source.comment_id,
      label: slug === null ? 'Recurso no disponible' : source.parent.title,
      detail: slug === null ? 'Recurso no disponible' : link.display_title,
      documentSlug: slug,
      taskReadableId: null,
    };
  }

  const slug = link.source_slug ?? null;
  return {
    id: link.source_document_id,
    label: slug === null ? 'Recurso no disponible' : link.source_title,
    detail: slug === null ? 'Recurso no disponible' : link.display_title,
    documentSlug: slug,
    taskReadableId: null,
  };
}

const rows = computed(() => props.backlinks.map(backlinkRow));

const heading = computed(() => {
  const n = props.backlinks.length;
  return `${n} linked reference${n === 1 ? '' : 's'}`;
});
</script>

<template>
  <div>
    <div
      style="
        font-size: 10px;
        font-weight: var(--fw-semibold);
        letter-spacing: 0.06em;
        text-transform: uppercase;
        color: var(--c-muted);
        margin-bottom: 8px;
      "
    >
      {{ heading }}
    </div>

    <LoadingState v-if="status === 'pending' && backlinks.length === 0" label="Loading backlinks…" />

    <ErrorState
      v-else-if="status === 'error'"
      title="Could not load backlinks"
      :hint="error ?? undefined"
      @retry="emit('retry')"
    />

    <p
      v-else-if="backlinks.length === 0"
      style="font-size: var(--fs-sm); color: var(--c-muted);"
    >
      No backlinks yet.
    </p>

    <button
      v-for="link in rows"
      :key="link.id"
      type="button"
      class="atl-card w-full text-left"
      :data-backlink-id="link.id"
      :disabled="link.documentSlug === null && link.taskReadableId === null"
      style="
        display: block;
        width: 100%;
        padding: 8px 10px;
        margin-bottom: 8px;
        border: 1px solid var(--c-border);
        border-radius: var(--r-sm);
        background: var(--c-raised);
        cursor: pointer;
      "
      @click="link.documentSlug !== null ? emit('navigate', link.documentSlug) : link.taskReadableId !== null && emit('navigate-task', link.taskReadableId)"
    >
      <div
        class="flex items-center"
        style="gap: 6px; margin-bottom: 4px; font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--c-foreground);"
      >
        <Icon name="file" :size="14" style="color: var(--c-muted); flex-shrink: 0;" />
        <span class="flex-1 truncate">{{ link.label }}</span>
      </div>
      <div style="font-size: var(--fs-xs); line-height: 1.5; color: var(--c-muted);">
        {{ link.detail }}
      </div>
    </button>
  </div>
</template>
