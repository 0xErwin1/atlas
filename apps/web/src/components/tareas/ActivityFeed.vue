<script setup lang="ts">
import { computed } from 'vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import type { ActivityEntryDto } from '@/stores/taskDetail';

const props = defineProps<{
  items: ActivityEntryDto[];
}>();

const VERB: Record<string, string> = {
  created: 'created this task',
  moved: 'moved the task',
  assigned: 'assigned the task',
  unassigned: 'unassigned the task',
  field_changed: 'changed a field',
  reference_added: 'added a reference',
  reference_removed: 'removed a reference',
  checklist_added: 'added a checklist item',
  checklist_updated: 'updated a checklist item',
  checklist_removed: 'removed a checklist item',
  checklist_promoted: 'promoted a checklist item to a task',
  deleted: 'deleted the task',
};

interface Row {
  id: string;
  name: string;
  isAgent: boolean;
  verb: string;
  when: string;
}

function isAgent(type: string): boolean {
  return type === 'api_key';
}

const rows = computed<Row[]>(() =>
  props.items.map((entry) => ({
    id: entry.id,
    name: entry.actor.display_name ?? (isAgent(entry.actor.type) ? 'Agent' : 'User'),
    isAgent: isAgent(entry.actor.type),
    verb: VERB[entry.kind] ?? entry.kind,
    when: entry.created_at,
  })),
);
</script>

<template>
  <ul
    v-if="rows.length > 0"
    class="flex flex-col"
    style="gap: 12px; list-style: none; margin: 0; padding: 0;"
  >
    <li
      v-for="row in rows"
      :key="row.id"
      class="flex items-start"
      style="gap: 8px;"
      :data-actor-kind="row.isAgent ? 'agent' : 'user'"
    >
      <Avatar :name="row.name" :agent="row.isAgent" :size="18" />

      <div class="flex flex-col" style="gap: 2px; min-width: 0;">
        <div class="flex items-center" style="gap: 6px; flex-wrap: wrap;">
          <span
            style="font-family: var(--font-mono); font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--c-foreground);"
          >
            {{ row.name }}
          </span>
          <AgentBadge v-if="row.isAgent" />
          <span style="font-size: var(--fs-sm); color: var(--c-foreground);">{{ row.verb }}</span>
        </div>
        <span
          style="font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--c-muted);"
        >
          {{ row.when }}
        </span>
      </div>
    </li>
  </ul>

  <p
    v-else
    style="font-size: var(--fs-sm); color: var(--c-muted);"
  >
    No activity yet.
  </p>
</template>
