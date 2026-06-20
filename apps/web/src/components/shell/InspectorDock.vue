<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import InspectorTabs from '@/components/ui/InspectorTabs.vue';
import type { InspectorTab } from '@/stores/ui';
import { useUiStore } from '@/stores/ui';

const ui = useUiStore();

const tabs: Array<{ id: InspectorTab; label: string; icon: string }> = [
  { id: 'properties', label: 'Properties', icon: 'columns' },
  { id: 'backlinks', label: 'Backlinks', icon: 'link' },
  { id: 'activity', label: 'Activity', icon: 'history' },
  { id: 'share', label: 'Share', icon: 'user' },
];

const collapsed = computed(() => !ui.inspectorOpen);
</script>

<template>
  <aside
    :style="`
      width: ${collapsed ? '40px' : '280px'};
      flex: 0 0 ${collapsed ? '40px' : '280px'};
      background-color: var(--c-panel);
      border-left: 1px solid var(--c-border);
      height: 100%;
      display: flex;
      flex-direction: column;
      overflow: hidden;
    `"
  >
    <template v-if="collapsed">
      <button
        type="button"
        title="Expand inspector"
        aria-label="Expand inspector"
        class="atl-gbtn"
        style="width: 28px; height: 28px; margin: 6px auto 0;"
        @click="ui.toggleInspector()"
      >
        <Icon name="panel-right" :size="15" />
      </button>

      <div
        aria-hidden="true"
        style="width: 20px; height: 1px; background: var(--c-border); margin: 6px auto;"
      />

      <div class="flex flex-col items-center">
        <button
          v-for="tab in tabs"
          :key="tab.id"
          type="button"
          :title="tab.label"
          :aria-label="tab.label"
          class="atl-gbtn"
          style="width: 28px; height: 30px; color: var(--c-muted);"
          @click="ui.setInspectorTab(tab.id); ui.toggleInspector()"
        >
          <Icon :name="tab.icon" :size="15" />
        </button>
      </div>
    </template>

    <template v-else>
      <InspectorTabs
        :tabs="tabs"
        :active="ui.inspectorTab"
        @update:active="(id) => ui.setInspectorTab(id as InspectorTab)"
      />

      <div class="flex-1 overflow-y-auto" style="padding: 10px;">
        <slot :tab="ui.inspectorTab" />
      </div>
    </template>
  </aside>
</template>
