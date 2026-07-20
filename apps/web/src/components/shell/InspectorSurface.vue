<script setup lang="ts">
import InspectorDock from '@/components/shell/InspectorDock.vue';
import BottomSheet from '@/components/ui/BottomSheet.vue';
import Icon from '@/components/ui/Icon.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import { type InspectorTab, useUiStore } from '@/stores/ui';

// The item inspector, rendered as a docked side panel on desktop and a "Details"
// bottom sheet on mobile. Consumers provide the per-tab content through the
// `panel` scoped slot, so this stays the single owner of the tab list and the
// desktop/mobile switch for every shell.
const INSPECTOR_TABS: Array<{ id: InspectorTab; label: string; icon: string }> = [
  { id: 'properties', label: 'Properties', icon: 'columns' },
  { id: 'backlinks', label: 'Backlinks', icon: 'link' },
  { id: 'comments', label: 'Comments', icon: 'message-square' },
  { id: 'activity', label: 'Activity', icon: 'history' },
  { id: 'share', label: 'Share', icon: 'user' },
];

const ui = useUiStore();
const { isMobile } = useBreakpoint();
</script>

<template>
  <BottomSheet
    v-if="isMobile"
    :open="ui.inspectorOpen"
    title="Details"
    @close="ui.toggleInspector()"
  >
    <div class="flex" style="gap: 2px; margin-bottom: 12px;">
      <button
        v-for="tab in INSPECTOR_TABS"
        :key="tab.id"
        type="button"
        class="flex items-center justify-center flex-1"
        :aria-selected="ui.inspectorTab === tab.id"
        :style="`
          gap: 6px;
          height: 34px;
          border: none;
          border-radius: var(--r-md);
          cursor: pointer;
          background: ${ui.inspectorTab === tab.id ? 'var(--c-selection)' : 'transparent'};
          color: ${ui.inspectorTab === tab.id ? 'var(--c-primary)' : 'var(--c-muted)'};
          font-size: var(--fs-sm);
        `"
        @click="ui.setInspectorTab(tab.id)"
      >
        <Icon :name="tab.icon" :size="15" />
      </button>
    </div>

    <slot name="panel" :tab="ui.inspectorTab" />
  </BottomSheet>

  <InspectorDock v-else>
    <template #default="{ tab }">
      <slot name="panel" :tab="tab" />
    </template>
  </InspectorDock>
</template>
