<script setup lang="ts">
import { computed } from 'vue';
import Avatar from '@/components/ui/Avatar.vue';
import Chip from '@/components/ui/Chip.vue';
import Icon from '@/components/ui/Icon.vue';
import MetaRow from '@/components/ui/MetaRow.vue';

const props = defineProps<{
  /** Frontmatter metadata extracted by useMarkdownDoc (REQ-W19). */
  meta: Record<string, unknown>;
}>();

type PropertyKind = 'status' | 'tags' | 'visibility' | 'person' | 'plain';

interface Property {
  key: string;
  kind: PropertyKind;
  text: string;
  tags: string[];
}

const PERSON_KEYS = new Set(['owner', 'author', 'assignee', 'reporter']);
const VISIBILITY_ICON: Record<string, string> = {
  private: 'lock',
  public: 'globe',
  workspace: 'users',
};

function stringify(value: unknown): string {
  if (value === null || value === undefined) return '';
  if (Array.isArray(value)) return value.map(stringify).join(', ');
  if (typeof value === 'object') return JSON.stringify(value);
  return String(value);
}

function toTags(value: unknown): string[] {
  if (Array.isArray(value)) return value.map(stringify).filter((t) => t.length > 0);
  return stringify(value)
    .split(',')
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
}

function classify(key: string, value: unknown): Property {
  const k = key.toLowerCase();

  if (k === 'tags' || k === 'labels') {
    return { key, kind: 'tags', text: '', tags: toTags(value) };
  }
  if (k === 'status') {
    return { key, kind: 'status', text: stringify(value), tags: [] };
  }
  if (k === 'visibility') {
    return { key, kind: 'visibility', text: stringify(value), tags: [] };
  }
  if (PERSON_KEYS.has(k)) {
    return { key, kind: 'person', text: stringify(value), tags: [] };
  }
  return { key, kind: 'plain', text: stringify(value), tags: [] };
}

const properties = computed<Property[]>(() =>
  Object.entries(props.meta).map(([key, value]) => classify(key, value)),
);

function visibilityIcon(value: string): string {
  return VISIBILITY_ICON[value.toLowerCase()] ?? 'eye';
}
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
        <template v-if="prop.kind === 'tags'">
          <Chip v-for="tag in prop.tags" :key="tag" tone="info">{{ tag }}</Chip>
          <span v-if="prop.tags.length === 0" style="color: var(--c-muted);">—</span>
        </template>

        <Chip v-else-if="prop.kind === 'status'" tone="info">{{ prop.text }}</Chip>

        <span v-else-if="prop.kind === 'visibility'" class="flex items-center" style="gap: 6px;">
          <Icon :name="visibilityIcon(prop.text)" :size="13" style="color: var(--c-muted);" />
          {{ prop.text }}
        </span>

        <span v-else-if="prop.kind === 'person'" class="flex items-center" style="gap: 6px;">
          <Avatar :name="prop.text" :size="16" />
          {{ prop.text }}
        </span>

        <span v-else style="word-break: break-word; color: var(--c-foreground);">{{ prop.text }}</span>
      </MetaRow>
    </div>
  </div>
</template>
