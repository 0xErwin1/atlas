<script setup lang="ts">
import { computed } from 'vue';
import MetaRow from '@/components/ui/MetaRow.vue';

const props = defineProps<{
  /** Frontmatter metadata extracted by useMarkdownDoc (REQ-W19). */
  meta: Record<string, unknown>;
}>();

interface Property {
  key: string;
  value: string;
}

function stringify(value: unknown): string {
  if (value === null || value === undefined) return '';
  if (Array.isArray(value)) return value.map(stringify).join(', ');
  if (typeof value === 'object') return JSON.stringify(value);
  return String(value);
}

const properties = computed<Property[]>(() =>
  Object.entries(props.meta).map(([key, value]) => ({ key, value: stringify(value) })),
);
</script>

<template>
  <div>
    <p
      v-if="properties.length === 0"
      style="font-size: var(--fs-sm); color: var(--c-muted);"
    >
      No properties.
    </p>

    <div
      v-else
      style="
        background: var(--c-raised);
        border: 1px solid var(--c-border);
        border-radius: var(--r-sm);
        padding: 10px 14px;
        display: flex;
        flex-direction: column;
        gap: 2px;
      "
    >
      <MetaRow
        v-for="prop in properties"
        :key="prop.key"
        :label="prop.key"
      >
        <span style="word-break: break-word; color: var(--c-foreground);">{{ prop.value }}</span>
      </MetaRow>
    </div>
  </div>
</template>
