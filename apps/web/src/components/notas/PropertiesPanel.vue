<script setup lang="ts">
import { computed } from 'vue';

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
      v-for="prop in properties"
      :key="prop.key"
      class="flex items-start gap-2"
      style="padding: 4px 0; border-bottom: 1px solid var(--c-border);"
    >
      <span
        style="
          width: 78px;
          flex-shrink: 0;
          font-family: var(--font-mono);
          font-size: var(--fs-xs);
          color: var(--c-muted);
        "
      >
        {{ prop.key }}
      </span>
      <span style="font-size: var(--fs-sm); color: var(--c-foreground); word-break: break-word;">
        {{ prop.value }}
      </span>
    </div>
  </div>
</template>
