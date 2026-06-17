<script setup lang="ts">
import { computed } from 'vue';
import ShareDialog from '@/components/share/ShareDialog.vue';
import AppRail from '@/components/shell/AppRail.vue';
import BannerToast from '@/components/shell/BannerToast.vue';
import ContextSidebar from '@/components/shell/ContextSidebar.vue';
import InspectorDock from '@/components/shell/InspectorDock.vue';
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

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');
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

    <InspectorDock>
      <template #default="{ tab }">
        <slot :name="`inspector-${tab}`" :tab="tab" />
      </template>
    </InspectorDock>

    <BannerToast />

    <ShareDialog
      :open="ui.shareOpen"
      :ws="ws"
      :resource-label="ui.shareResourceLabel"
      @close="ui.closeShare()"
    />
  </div>
</template>
