<script setup lang="ts">
/**
 * Filter panel for the tasks board. Renders four MultiSelect facets — Status,
 * Priority, Labels and Assignee — bound to the session-only `ui.taskFilter`
 * state, and a clear-all affordance shown only while a filter is active. The
 * panel is meant to live inside the toolbar's filter Popover (the trigger is
 * owned by Tasks.vue, mirroring the adjacent Group-by control); it owns only the
 * facet UI and reads option sources from the boards, label-color and workspace
 * stores.
 */
import { computed, type WritableComputedRef } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import MultiSelect, { type MultiSelectOption } from '@/components/ui/MultiSelect.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import { resolveColumnSwatchId } from '@/lib/columnColor';
import { swatchById } from '@/lib/swatches';
import { useBoardsStore } from '@/stores/boards';
import { useLabelColorsStore } from '@/stores/labelColors';
import { useTagsStore } from '@/stores/tags';
import { type TaskFilterState, useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const ui = useUiStore();
const boards = useBoardsStore();
const labelColors = useLabelColorsStore();
const tags = useTagsStore();
const workspace = useWorkspaceStore();

const PRIORITY_OPTIONS: MultiSelectOption[] = [
  { value: 'urgent', label: 'Urgent', dot: 'var(--c-danger)' },
  { value: 'high', label: 'High', dot: 'var(--c-primary)' },
  { value: 'medium', label: 'Medium', dot: 'var(--c-info)' },
  { value: 'low', label: 'Low', dot: 'var(--c-muted)' },
];

const statusOptions = computed<MultiSelectOption[]>(() =>
  boards.columns.map((column) => ({
    value: column.id,
    label: column.name,
    dot: swatchById(resolveColumnSwatchId(column)).fg,
  })),
);

const labelOptions = computed<MultiSelectOption[]>(() =>
  labelColors.tagNames.map((name) => ({
    value: name,
    label: name,
    dot: swatchById(tags.colorFor(name)).fg,
  })),
);

const assigneeOptions = computed<MultiSelectOption[]>(() =>
  workspace.members.map((member) => ({
    value: member.id,
    label: member.display,
    icon: member.principal_type === 'api_key' ? 'bot' : 'user',
  })),
);

// Each facet writes its own slice of the filter. The setter clones the current
// state and replaces just the targeted dimension so the store ref's arrays are
// never mutated in place, keeping reactivity and the immutable-update contract.
function facetModel(key: keyof TaskFilterState): WritableComputedRef<string[]> {
  return computed<string[]>({
    get: () => ui.taskFilter[key],
    set: (values) => {
      ui.setTaskFilter({ ...ui.taskFilter, [key]: values });
    },
  });
}

const statusModel = facetModel('statuses');
const priorityModel = facetModel('priorities');
const labelModel = facetModel('labels');
const assigneeModel = facetModel('assigneeIds');
</script>

<template>
  <div :style="{ width: '236px', padding: '4px 0 6px' }">
    <div
      class="flex items-center"
      :style="{
        justifyContent: 'space-between',
        padding: '6px 10px 2px',
      }"
    >
      <span
        :style="{
          fontSize: 'var(--fs-sm)',
          fontWeight: 'var(--fw-semibold)',
          color: 'var(--c-foreground)',
        }"
      >
        Filter tasks
      </span>
      <button
        v-if="ui.hasActiveFilter"
        type="button"
        class="atl-clear-filters inline-flex items-center cursor-pointer"
        :style="{
          gap: '4px',
          padding: '2px 5px',
          background: 'transparent',
          border: 'none',
          borderRadius: 'var(--r-sm)',
          fontSize: '11px',
          color: 'var(--c-muted)',
        }"
        @click="ui.clearTaskFilter()"
      >
        <Icon name="x" :size="11" />
        Clear all
      </button>
    </div>

    <SectionLabel>Status</SectionLabel>
    <div :style="{ padding: '0 10px 4px' }">
      <MultiSelect v-model="statusModel" :options="statusOptions" placeholder="Any status" />
    </div>

    <SectionLabel>Priority</SectionLabel>
    <div :style="{ padding: '0 10px 4px' }">
      <MultiSelect v-model="priorityModel" :options="PRIORITY_OPTIONS" placeholder="Any priority" />
    </div>

    <SectionLabel>Labels</SectionLabel>
    <div :style="{ padding: '0 10px 4px' }">
      <MultiSelect v-model="labelModel" :options="labelOptions" icon="tag" placeholder="Any label" />
    </div>

    <SectionLabel>Assignee</SectionLabel>
    <div :style="{ padding: '0 10px 2px' }">
      <MultiSelect v-model="assigneeModel" :options="assigneeOptions" icon="user" placeholder="Anyone" />
    </div>
  </div>
</template>

<style scoped>
.atl-clear-filters:hover {
  background: rgba(179, 177, 173, 0.06);
  color: var(--c-foreground);
}
</style>
