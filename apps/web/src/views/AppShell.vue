<script setup lang="ts">
import AppRail from '@/components/shell/AppRail.vue';
import BannerToast from '@/components/shell/BannerToast.vue';
import ContextSidebar from '@/components/shell/ContextSidebar.vue';
import InspectorDock from '@/components/shell/InspectorDock.vue';
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
  </div>
</template>
