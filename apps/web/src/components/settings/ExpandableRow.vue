<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';

const props = withDefaults(
  defineProps<{
    expanded: boolean;
    expandable?: boolean;
  }>(),
  {
    expandable: true,
  },
);

const emit = defineEmits<{ toggle: [] }>();

// Fall-through attrs (data hooks, height/`--erow-actions-basis` overrides) land
// on the row element rather than being dropped, because this component renders
// the row and its panel as sibling roots.
defineOptions({ inheritAttrs: false });

const clickable = computed(() => props.expandable);

function onRowClick(): void {
  if (clickable.value) emit('toggle');
}
</script>

<template>
  <div
    v-bind="$attrs"
    class="atl-erow"
    :class="{ 'atl-erow--clickable': clickable, 'atl-erow--expanded': expanded }"
    data-row
    @click="onRowClick"
  >
    <slot name="summary" />

    <div class="atl-erow-actions" @click.stop>
      <button
        v-if="clickable"
        type="button"
        class="atl-rowact"
        data-action="manage"
        @click="emit('toggle')"
      >
        <Icon name="sliders-horizontal" :size="13" />
        Manage
        <Icon :name="expanded ? 'chevron-up' : 'chevron-down'" :size="13" />
      </button>
      <slot name="actions" />
    </div>
  </div>

  <div v-if="expanded" class="atl-erow-panel" data-row-panel>
    <slot name="panel" />
  </div>
</template>

<style scoped>
.atl-erow {
  display: flex;
  align-items: center;
  padding: 0 12px;
  border-top: 1px solid var(--c-border);
  transition: background 0.1s;
}

.atl-erow--clickable {
  cursor: pointer;
}

.atl-erow:hover {
  background: var(--c-raised);
}

.atl-erow--expanded {
  background: var(--c-raised);
}

.atl-erow-actions {
  flex: 0 0 var(--erow-actions-basis, auto);
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
}

.atl-rowact {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 24px;
  padding: 0 8px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-foreground);
  cursor: pointer;
  font-size: 12px;
}

.atl-rowact:hover {
  background: var(--c-background);
}

.atl-erow-panel {
  border-top: 1px solid var(--c-border);
  background: var(--c-background);
  padding: 12px 40px;
}
</style>
