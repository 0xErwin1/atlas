<script setup lang="ts">
import { useSlots } from 'vue';
import InspectorSurface from '@/components/shell/InspectorSurface.vue';
import EmptyState from '@/components/states/EmptyState.vue';

// The content region of a Docs view: the main pane plus, when the view provides
// inspector slots, the shared inspector surface. The persistent app frame (rail,
// sidebar, tab bar, dialogs) lives in WorkspaceShell, so a view renders only this.
const slots = useSlots();

const hasInspector = (): boolean => Object.keys(slots).some((name) => name.startsWith('inspector-'));
</script>

<template>
  <main
    class="flex flex-col flex-1 min-w-0 overflow-hidden"
    style="background-color: var(--c-background);"
  >
    <slot />
  </main>

  <InspectorSurface v-if="hasInspector()">
    <template #panel="{ tab }">
      <slot :name="`inspector-${tab}`" :tab="tab">
        <EmptyState
          icon="panel-right"
          title="Nothing to show"
          hint="This panel has no content for the current view."
        />
      </slot>
    </template>
  </InspectorSurface>
</template>
