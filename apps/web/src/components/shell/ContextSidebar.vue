<script setup lang="ts">
import WorkspaceSwitcher from '@/components/shell/WorkspaceSwitcher.vue';
</script>

<template>
  <aside
    style="
      width: 264px;
      flex: 0 0 264px;
      min-width: 248px;
      background-color: var(--c-panel);
      border-right: 1px solid var(--c-border);
      height: 100%;
      display: flex;
      flex-direction: column;
      overflow: hidden;
    "
  >
    <div
      class="flex items-center"
      style="
        height: 36px;
        padding: 0 6px 0 6px;
        gap: 8px;
        border-bottom: 1px solid var(--c-border);
        flex-shrink: 0;
      "
    >
      <div class="flex-1 min-w-0">
        <WorkspaceSwitcher />
      </div>

      <div class="flex items-center" style="gap: 2px;">
        <slot name="header-actions" />
      </div>
    </div>

    <div class="flex-1 min-h-0 overflow-hidden">
      <slot />
    </div>

    <div
      v-if="$slots.footer"
      style="border-top: 1px solid var(--c-border); padding: 8px; flex-shrink: 0;"
    >
      <slot name="footer" />
    </div>
  </aside>
</template>
