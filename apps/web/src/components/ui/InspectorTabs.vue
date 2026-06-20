<script setup lang="ts">
/**
 * Shared inspector/preview tab strip: a 36px header of tabs with the active one
 * marked by an inset primary underline. The single source for every inspector
 * tab bar (the editor inspector, the task inspector, the search preview), so
 * they can no longer drift apart.
 *
 * Renders icon-only cells when there is more than one tab and every tab carries
 * an `icon` (the design keeps a multi-tab strip on one line); otherwise it falls
 * back to text labels.
 */
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';

interface Tab {
  id: string;
  label: string;
  icon?: string;
}

const props = defineProps<{
  tabs: Tab[];
}>();

const active = defineModel<string>('active', { required: true });

const iconOnly = computed(() => props.tabs.length > 1 && props.tabs.every((tab) => Boolean(tab.icon)));
</script>

<template>
  <div class="atl-tabstrip" :class="{ 'icon-only': iconOnly }">
    <button
      v-for="tab in tabs"
      :key="tab.id"
      type="button"
      class="atl-itab"
      :class="{ on: active === tab.id, 'icon-cell': iconOnly }"
      :aria-selected="active === tab.id"
      :title="iconOnly ? tab.label : undefined"
      @click="active = tab.id"
    >
      <Icon v-if="iconOnly && tab.icon" :name="tab.icon" :size="15" />
      <template v-else>{{ tab.label }}</template>
    </button>
    <slot name="end" />
  </div>
</template>

<style scoped>
.atl-tabstrip {
  display: flex;
  align-items: flex-end;
  height: 36px;
  flex: 0 0 36px;
  padding: 0 4px;
  border-bottom: 1px solid var(--c-border);
}

.atl-tabstrip.icon-only {
  gap: 2px;
}

.atl-itab {
  height: 28px;
  padding: 0 9px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: var(--fs-sm);
  font-weight: var(--fw-medium);
  color: var(--c-muted);
}

.atl-itab:hover {
  color: var(--c-foreground);
}

.atl-itab.on {
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
  box-shadow: inset 0 -2px 0 var(--c-primary);
}

.atl-itab.icon-cell {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  padding: 0;
  color: var(--c-muted);
}

.atl-itab.icon-cell.on {
  color: var(--c-primary);
}
</style>
