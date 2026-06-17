<script setup lang="ts">
import Crumb from '@/components/ui/Crumb.vue';
import Icon from '@/components/ui/Icon.vue';

withDefaults(
  defineProps<{
    breadcrumbs?: string[];
    dirty?: boolean;
  }>(),
  {
    breadcrumbs: () => [],
    dirty: false,
  },
);
</script>

<template>
  <div
    class="flex items-center"
    style="
      height: 32px;
      flex: 0 0 32px;
      gap: 10px;
      padding: 0 8px 0 12px;
      background-color: var(--c-panel);
      border-bottom: 1px solid var(--c-border);
    "
  >
    <Crumb :parts="breadcrumbs" />

    <div style="flex: 1;" />

    <button type="button" class="atl-gbtn" title="History" aria-label="History">
      <Icon name="history" :size="14" />
    </button>
    <button type="button" class="atl-gbtn" title="Share" aria-label="Share">
      <Icon name="user" :size="14" />
    </button>
    <button type="button" class="atl-gbtn" title="More" aria-label="More">
      <Icon name="ellipsis" :size="14" />
    </button>

    <div aria-hidden="true" style="width: 1px; height: 18px; background: var(--c-border);" />

    <slot />
  </div>
</template>
