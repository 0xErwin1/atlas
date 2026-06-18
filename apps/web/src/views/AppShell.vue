<script setup lang="ts">
import { computed, useSlots } from 'vue';
import SettingsModal from '@/components/settings/SettingsModal.vue';
import ShareDialog from '@/components/share/ShareDialog.vue';
import AppRail from '@/components/shell/AppRail.vue';
import BannerToast from '@/components/shell/BannerToast.vue';
import ContextSidebar from '@/components/shell/ContextSidebar.vue';
import InspectorDock from '@/components/shell/InspectorDock.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

withDefaults(
  defineProps<{
    sidebarTitle?: string;
    sidebarIcon?: string;
  }>(),
  {
    sidebarTitle: 'Explorer',
    sidebarIcon: '',
  },
);

const ui = useUiStore();
const workspace = useWorkspaceStore();
const slots = useSlots();

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

// The inspector tabs are item-scoped (properties, backlinks, …). A view that
// provides no inspector slot — e.g. the board — has nothing to show there, so the
// dock is hidden rather than opening to a blank panel.
const hasInspector = computed(() => Object.keys(slots).some((name) => name.startsWith('inspector-')));
</script>

<template>
  <div
    class="flex"
    style="height: 100vh; overflow: hidden; background-color: var(--c-background);"
  >
    <AppRail />

    <ContextSidebar :title="sidebarTitle" :icon="sidebarIcon">
      <template #header-actions>
        <slot name="sidebar-actions" />
      </template>
      <slot name="sidebar" />
      <template v-if="$slots['sidebar-footer']" #footer>
        <slot name="sidebar-footer" />
      </template>
    </ContextSidebar>

    <main
      class="flex flex-col flex-1 min-w-0 overflow-hidden"
      style="background-color: var(--c-background);"
    >
      <slot />
    </main>

    <InspectorDock v-if="hasInspector">
      <template #default="{ tab }">
        <slot :name="`inspector-${tab}`" :tab="tab">
          <EmptyState
            icon="panel-right"
            title="Nothing to show"
            hint="This panel has no content for the current view."
          />
        </slot>
      </template>
    </InspectorDock>

    <BannerToast />

    <ShareDialog
      :open="ui.shareOpen"
      :ws="ws"
      :resource-label="ui.shareResourceLabel"
      @close="ui.closeShare()"
    />

    <SettingsModal />
  </div>
</template>
