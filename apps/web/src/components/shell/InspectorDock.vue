<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import type { InspectorTab } from '@/stores/ui';
import { useUiStore } from '@/stores/ui';

const ui = useUiStore();

const tabs: Array<{ id: InspectorTab; label: string; icon: string }> = [
  { id: 'properties', label: 'Properties', icon: 'hash' },
  { id: 'backlinks', label: 'Backlinks', icon: 'link' },
  { id: 'activity', label: 'Activity', icon: 'clock' },
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
      <div
        class="flex items-end"
        style="
          height: 36px;
          flex: 0 0 36px;
          border-bottom: 1px solid var(--c-border);
          padding: 0 4px;
        "
      >
        <button
          v-for="tab in tabs"
          :key="tab.id"
          type="button"
          class="atl-itab flex items-center"
          :aria-selected="ui.inspectorTab === tab.id"
          :style="`
            padding: 0 9px;
            height: 28px;
            border: none;
            cursor: pointer;
            background: transparent;
            font-size: var(--fs-sm);
            white-space: nowrap;
            color: ${ui.inspectorTab === tab.id ? 'var(--c-foreground)' : 'var(--c-muted)'};
            font-weight: ${ui.inspectorTab === tab.id ? 'var(--fw-bold)' : 'var(--fw-medium)'};
            box-shadow: ${ui.inspectorTab === tab.id ? 'inset 0 -2px 0 var(--c-primary)' : 'none'};
          `"
          @click="ui.setInspectorTab(tab.id)"
        >
          {{ tab.label }}
        </button>

        <button
          type="button"
          title="Collapse inspector"
          aria-label="Collapse inspector"
          class="atl-gbtn"
          style="margin-left: auto; width: 28px; height: 28px; align-self: center;"
          @click="ui.toggleInspector()"
        >
          <Icon name="panel-right-close" :size="15" />
        </button>
      </div>

      <div class="flex-1 overflow-y-auto" style="padding: 10px;">
        <slot :tab="ui.inspectorTab" />
      </div>
    </template>
  </aside>
</template>
