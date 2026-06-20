<script setup lang="ts">
import { computed } from 'vue';
import type { components } from '@/api/types.d.ts';
import Icon from '@/components/ui/Icon.vue';

type Backlink = components['schemas']['Page_BacklinkDto']['items'][number];

const props = defineProps<{
  backlinks: Backlink[];
}>();

const emit = defineEmits<{
  navigate: [slug: string];
}>();

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

    <p
      v-if="backlinks.length === 0"
      style="font-size: var(--fs-sm); color: var(--c-muted);"
    >
      No backlinks yet.
    </p>

    <button
      v-for="link in backlinks"
      :key="link.source_document_id"
      type="button"
      class="atl-card w-full text-left"
      :disabled="link.source_slug === null || link.source_slug === undefined"
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
      @click="link.source_slug && emit('navigate', link.source_slug)"
    >
      <div
        class="flex items-center"
        style="gap: 6px; margin-bottom: 4px; font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--c-foreground);"
      >
        <Icon name="file" :size="14" style="color: var(--c-muted); flex-shrink: 0;" />
        <span class="flex-1 truncate">{{ link.source_title }}</span>
      </div>
      <div style="font-size: var(--fs-xs); line-height: 1.5; color: var(--c-muted);">
        {{ link.display_title }}
      </div>
    </button>
  </div>
</template>
