<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import type { InspectorTab } from '@/stores/ui';
import { useUiStore } from '@/stores/ui';

const ui = useUiStore();

const tabs: Array<{ id: InspectorTab; label: string; icon: string }> = [
  { id: 'properties', label: 'Properties', icon: 'list' },
  { id: 'backlinks', label: 'Backlinks', icon: 'link' },
  { id: 'activity', label: 'Activity', icon: 'activity' },
  { id: 'share', label: 'Share', icon: 'share-2' },
];

const collapsed = computed(() => !ui.inspectorOpen);
</script>

<template>
  <aside
    :style="`
      width: ${collapsed ? '40px' : '290px'};
      min-width: ${collapsed ? '40px' : '280px'};
      max-width: ${collapsed ? '40px' : '300px'};
      background-color: var(--c-panel);
      border-left: 1px solid var(--c-border);
      flex-shrink: 0;
      height: 100%;
      display: flex;
      flex-direction: column;
      overflow: hidden;
      transition: width 0.15s ease, min-width 0.15s ease;
    `"
  >
    <div
      v-if="collapsed"
      class="flex flex-col items-center gap-1 pt-2"
    >
      <button
        v-for="tab in tabs"
        :key="tab.id"
        type="button"
        :title="tab.label"
        :aria-label="tab.label"
        class="flex items-center justify-center"
        style="
          width: 40px;
          height: 40px;
          border: none;
          cursor: pointer;
          border-radius: var(--r-md);
          background: transparent;
          color: var(--c-muted);
        "
        @click="ui.setInspectorTab(tab.id); ui.toggleInspector()"
      >
        <Icon :name="tab.icon" :size="16" />
      </button>
    </div>

    <template v-else>
      <div
        class="flex items-center"
        style="
          height: 36px;
          border-bottom: 1px solid var(--c-border);
          flex-shrink: 0;
          overflow-x: auto;
        "
      >
        <button
          v-for="tab in tabs"
          :key="tab.id"
          type="button"
          :aria-selected="ui.inspectorTab === tab.id"
          class="flex items-center justify-center"
          :style="`
            height: 36px;
            padding: 0 12px;
            border: none;
            cursor: pointer;
            background: transparent;
            font-family: var(--font-mono);
            font-size: var(--fs-xs);
            white-space: nowrap;
            position: relative;
            color: ${ui.inspectorTab === tab.id ? 'var(--c-foreground)' : 'var(--c-muted)'};
            font-weight: ${ui.inspectorTab === tab.id ? 'var(--fw-bold)' : 'var(--fw-normal)'};
          `"
          @click="ui.setInspectorTab(tab.id)"
        >
          {{ tab.label }}
          <span
            v-if="ui.inspectorTab === tab.id"
            style="
              position: absolute;
              bottom: 0;
              left: 0;
              right: 0;
              height: 2px;
              background-color: var(--c-primary);
              border-radius: 1px 1px 0 0;
            "
            aria-hidden="true"
          />
        </button>

        <button
          type="button"
          title="Collapse inspector"
          aria-label="Collapse inspector"
          style="
            margin-left: auto;
            width: 32px;
            height: 32px;
            border: none;
            cursor: pointer;
            border-radius: var(--r-md);
            background: transparent;
            color: var(--c-muted);
            flex-shrink: 0;
            display: flex;
            align-items: center;
            justify-content: center;
          "
          @click="ui.toggleInspector()"
        >
          <Icon name="panel-right-close" :size="14" />
        </button>
      </div>

      <div
        class="flex-1 overflow-y-auto"
        style="padding: 12px;"
      >
        <slot :tab="ui.inspectorTab" />
      </div>
    </template>
  </aside>
</template>
