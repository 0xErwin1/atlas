<script setup lang="ts">
import { computed } from 'vue';
import { type RouteLocationRaw, RouterLink } from 'vue-router';
import Chip, { type ChipTone } from '@/components/ui/Chip.vue';
import Icon from '@/components/ui/Icon.vue';
import type { ReferenceDto } from '@/stores/taskDetail';

const props = defineProps<{
  references: ReferenceDto[];
}>();

const emit = defineEmits<{
  remove: [referenceId: string];
}>();

const KIND_TONE: Record<string, ChipTone> = {
  relates: 'info',
  blocks: 'danger',
  parent: 'agent',
  spec: 'success',
  docs: 'warning',
};

interface Row {
  id: string;
  kind: string;
  tone: ChipTone;
  target: string;
  resolved: boolean;
  to: RouteLocationRaw | null;
}

// A resolved reference points at either a task (by readable ID) or a document (by
// id, which the notes route accepts as its slug). A broken reference has no live
// target to open, so it stays plain text.
function targetRoute(r: ReferenceDto): RouteLocationRaw | null {
  if (!r.target_resolved) return null;
  if (r.target_readable_id != null) {
    return { name: 'task-detail', params: { readableId: r.target_readable_id } };
  }
  if (r.target_document_id != null) {
    return { name: 'notes', params: { slug: r.target_document_id } };
  }
  return null;
}

const rows = computed<Row[]>(() =>
  props.references.map((r) => ({
    id: r.id,
    kind: r.kind,
    tone: KIND_TONE[r.kind] ?? 'neutral',
    target: r.target_readable_id ?? r.target_title ?? r.target_document_id ?? 'unknown',
    resolved: r.target_resolved,
    to: targetRoute(r),
  })),
);
</script>

<template>
  <div class="flex flex-col" style="gap: 6px;">
    <div
      v-for="row in rows"
      :key="row.id"
      class="group flex items-center"
      style="gap: 8px;"
      :data-reference-id="row.id"
    >
      <Chip :tone="row.tone">{{ row.kind }}</Chip>
      <component
        :is="row.to ? RouterLink : 'span'"
        :to="row.to ?? undefined"
        class="atl-ref-target flex-1 min-w-0 truncate"
        :style="{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-sm)',
          color: row.resolved ? 'var(--c-foreground)' : 'var(--c-danger)',
          textDecoration: row.resolved ? 'none' : 'line-through',
        }"
        :title="row.resolved ? row.target : `${row.target} (broken)`"
      >
        {{ row.target }}
      </component>
      <button
        type="button"
        :aria-label="`Remove reference ${row.target}`"
        class="inline-flex items-center justify-center cursor-pointer opacity-0 group-hover:opacity-100"
        style="width: 16px; height: 16px; border: none; background: transparent; color: var(--c-muted); padding: 0;"
        @click="emit('remove', row.id)"
      >
        <Icon name="x" :size="13" />
      </button>
    </div>

    <p
      v-if="rows.length === 0"
      style="font-size: var(--fs-sm); color: var(--c-muted);"
    >
      No references.
    </p>
  </div>
</template>

<style scoped>
a.atl-ref-target {
  cursor: pointer;
}

a.atl-ref-target:hover {
  text-decoration: underline;
}
</style>
