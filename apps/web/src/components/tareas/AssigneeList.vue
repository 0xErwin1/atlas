<script setup lang="ts">
import { computed } from 'vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import Icon from '@/components/ui/Icon.vue';
import type { AssigneeDto } from '@/stores/taskDetail';

const props = defineProps<{
  assignees: AssigneeDto[];
}>();

const emit = defineEmits<{
  remove: [assigneeType: string, assigneeId: string];
}>();

interface Row {
  id: string;
  type: string;
  name: string;
  isAgent: boolean;
}

const rows = computed<Row[]>(() =>
  props.assignees.map((a) => ({
    id: a.assignee.id,
    type: a.assignee.type,
    name: a.assignee.display_name ?? (a.assignee.type === 'api_key' ? 'Agent' : 'User'),
    isAgent: a.assignee.type === 'api_key',
  })),
);
</script>

<template>
  <div class="flex items-center flex-wrap" style="gap: 6px;">
    <span
      v-for="row in rows"
      :key="`${row.type}:${row.id}`"
      class="group inline-flex items-center"
      style="gap: 5px; padding: 2px 6px 2px 2px; border-radius: var(--r-full); background: var(--c-raised); border: 1px solid var(--c-border);"
      :data-assignee-kind="row.isAgent ? 'agent' : 'user'"
    >
      <Avatar :name="row.name" :agent="row.isAgent" :size="18" />
      <span style="font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--c-foreground);">
        {{ row.name }}
      </span>
      <AgentBadge v-if="row.isAgent" />
      <button
        type="button"
        :aria-label="`Remove ${row.name}`"
        class="inline-flex items-center justify-center cursor-pointer opacity-0 group-hover:opacity-100"
        style="width: 14px; height: 14px; border: none; background: transparent; color: var(--c-muted); padding: 0;"
        @click="emit('remove', row.type, row.id)"
      >
        <Icon name="x" :size="12" />
      </button>
    </span>

    <span
      v-if="rows.length === 0"
      style="font-size: var(--fs-sm); color: var(--c-muted);"
    >
      Unassigned
    </span>
  </div>
</template>
