<script setup lang="ts">
import { computed, ref } from 'vue';
import ShareDialog, { type Visibility } from '@/components/share/ShareDialog.vue';
import AppRail from '@/components/shell/AppRail.vue';
import BannerToast from '@/components/shell/BannerToast.vue';
import ContextSidebar from '@/components/shell/ContextSidebar.vue';
import InspectorDock from '@/components/shell/InspectorDock.vue';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const ui = useUiStore();
const workspace = useWorkspaceStore();

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');
const visibility = ref<Visibility>('workspace');
</script>

<template>
  <div
    class="flex"
    style="height: 100vh; overflow: hidden; background-color: var(--c-background);"
  >
    <AppRail />

    <ContextSidebar>
      <template #header-actions>
        <slot name="sidebar-actions" />
      </template>
      <slot name="sidebar" />
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
      v-model:visibility="visibility"
      @close="ui.closeShare()"
    />
  </div>
</template>
