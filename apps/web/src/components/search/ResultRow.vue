<script setup lang="ts">
import { computed } from 'vue';
import Chip from '@/components/ui/Chip.vue';
import Icon from '@/components/ui/Icon.vue';
import { relativeTime } from '@/lib/relativeTime';
import { sanitizeSnippet } from '@/lib/sanitize';
import type { SearchHitDto } from '@/stores/search';

const props = withDefaults(
  defineProps<{
    hit: SearchHitDto;
    active?: boolean;
  }>(),
  {
    active: false,
  },
);

const isTask = computed(() => props.hit.kind === 'task');

const kindLabel = computed(() => (isTask.value ? 'TASK' : 'NOTE'));

const iconName = computed(() => (isTask.value ? 'tasks' : 'file'));

/**
 * Snippet HTML is sanitized to only allow <mark> before it reaches v-html.
 * sanitizeSnippet strips every other tag, so a malicious snippet (e.g.
 * <img onerror> or <script>) cannot inject anything (REQ-W25).
 */
const safeSnippet = computed(() => (props.hit.snippet ? sanitizeSnippet(props.hit.snippet) : null));

const updatedLabel = computed(() => relativeTime(props.hit.updated_at));
</script>

<template>
  <button
    type="button"
    class="atl-result flex w-full text-left items-start"
    data-kind="search-result"
    :data-active="active ? 'true' : 'false'"
    :style="{
      gap: '11px',
      padding: '11px 13px',
      cursor: 'pointer',
      borderBottom: '1px solid var(--c-border)',
      background: active ? 'var(--c-selection)' : 'transparent',
      boxShadow: active ? 'inset 2px 0 0 var(--c-primary)' : 'none',
    }"
  >
    <Icon
      :name="iconName"
      :size="16"
      :style="{
        color: active ? 'var(--c-primary)' : 'var(--c-muted)',
        flex: '0 0 auto',
        marginTop: '2px',
      }"
    />

    <span class="flex-1 min-w-0">
      <span
        class="block truncate"
        :style="{
          fontSize: 'var(--fs-lg)',
          fontWeight: 'var(--fw-semibold)',
          color: 'var(--c-foreground)',
          marginBottom: '3px',
        }"
      >
        <span
          v-if="isTask && hit.readable_id"
          :style="{ fontFamily: 'var(--font-mono)', color: 'var(--c-muted)', marginRight: '6px' }"
        >{{ hit.readable_id }}</span>{{ hit.title }}
      </span>

      <!-- eslint-disable-next-line vue/no-v-html -- sanitizeSnippet allows only <mark> (REQ-W25) -->
      <span
        v-if="safeSnippet"
        class="block"
        data-testid="snippet"
        :style="{
          fontSize: '12.5px',
          color: 'var(--c-muted)',
          lineHeight: '1.4',
          marginBottom: '6px',
        }"
        v-html="safeSnippet"
      />

      <span class="flex items-center flex-wrap" :style="{ gap: '10px' }">
        <Chip tone="neutral">{{ kindLabel }}</Chip>
        <span
          v-if="hit.project_slug"
          class="flex items-center"
          :style="{ gap: '4px', fontSize: 'var(--fs-xs)', color: 'var(--c-muted)' }"
        >
          <Icon name="folder" :size="12" />
          {{ hit.project_slug }}
        </span>
        <span
          class="flex items-center"
          :style="{ gap: '5px', fontSize: 'var(--fs-xs)', color: 'var(--c-muted)' }"
        >
          <Icon name="clock" :size="12" />
          {{ updatedLabel }}
        </span>
      </span>
    </span>
  </button>
</template>

<style scoped>
.atl-result :deep(mark) {
  background: rgba(255, 180, 84, 0.25);
  color: var(--c-foreground);
  border-radius: 2px;
  padding: 0 2px;
}
</style>
