<script setup lang="ts">
import { computed } from 'vue';
import Btn from '@/components/ui/Btn.vue';
import Chip from '@/components/ui/Chip.vue';
import Crumb from '@/components/ui/Crumb.vue';
import Icon from '@/components/ui/Icon.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import { relativeTime } from '@/lib/relativeTime';
import { sanitizeSnippet } from '@/lib/sanitize';
import type { SearchHitDto } from '@/stores/search';

const props = defineProps<{
  hit: SearchHitDto;
}>();

const emit = defineEmits<{
  open: [hit: SearchHitDto];
}>();

const isTask = computed(() => props.hit.kind === 'task');
const kindLabel = computed(() => (isTask.value ? 'TASK' : 'NOTE'));
const iconName = computed(() => (isTask.value ? 'square-check-big' : 'file-text'));

const crumbParts = computed(() => {
  const parts = ['Atlas'];
  if (props.hit.project_slug) parts.push(props.hit.project_slug);
  return parts;
});

const safeSnippet = computed(() => (props.hit.snippet ? sanitizeSnippet(props.hit.snippet) : null));

const updatedLabel = computed(() => relativeTime(props.hit.updated_at));
</script>

<template>
  <aside class="atl-search-preview">
    <div class="flex items-center" style="gap: 7px; margin-bottom: 8px;">
      <Icon :name="iconName" :size="15" style="color: var(--c-muted);" />
      <Chip tone="neutral">{{ kindLabel }}</Chip>
      <span
        v-if="isTask && hit.readable_id"
        style="font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--c-muted);"
      >
        {{ hit.readable_id }}
      </span>
    </div>

    <h2 style="font-size: var(--fs-xl); font-weight: var(--fw-bold); color: var(--c-foreground); margin: 0 0 6px; line-height: 1.25;">
      {{ hit.title }}
    </h2>

    <Crumb :parts="crumbParts" />

    <!-- eslint-disable-next-line vue/no-v-html -- sanitizeSnippet allows only <mark> (REQ-W25) -->
    <div
      v-if="safeSnippet"
      class="atl-search-preview-snip"
      v-html="safeSnippet"
    />
    <p v-else class="atl-search-preview-snip" style="color: var(--c-muted);">
      No preview available for this match.
    </p>

    <Btn variant="primary" style="width: 100%; margin-top: 4px;" @click="emit('open', hit)">
      <Icon name="arrow-right" :size="14" />
      {{ isTask ? 'Open task' : 'Open note' }}
    </Btn>

    <SectionLabel flush style="margin-top: 18px;">Details</SectionLabel>
    <div class="flex items-center" style="gap: 6px; font-size: var(--fs-sm); color: var(--c-muted);">
      <Icon name="clock" :size="13" />
      Updated {{ updatedLabel }}
    </div>
  </aside>
</template>

<style scoped>
.atl-search-preview {
  width: 300px;
  flex: 0 0 300px;
  padding: 16px;
  overflow-y: auto;
  background: var(--c-panel);
  border-left: 1px solid var(--c-border);
}

.atl-search-preview-snip {
  margin: 12px 0;
  padding: 10px 12px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  font-size: 12.5px;
  line-height: 1.5;
  color: var(--c-muted);
}

.atl-search-preview-snip :deep(mark) {
  background: rgba(255, 180, 84, 0.25);
  color: var(--c-foreground);
  border-radius: 2px;
  padding: 0 2px;
}
</style>
