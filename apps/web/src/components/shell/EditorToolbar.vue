<script setup lang="ts">
import Crumb from '@/components/ui/Crumb.vue';
import Icon from '@/components/ui/Icon.vue';
import { useUiStore } from '@/stores/ui';

const props = withDefaults(
  defineProps<{
    breadcrumbs?: string[];
    dirty?: boolean;
    /** When set, a Share button opens the share dialog for this resource. */
    shareLabel?: string;
  }>(),
  {
    breadcrumbs: () => [],
    dirty: false,
    shareLabel: '',
  },
);

const ui = useUiStore();

function openShare() {
  if (props.shareLabel !== '') ui.openShare(props.shareLabel);
}
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
    <button
      v-if="ui.sidebarCollapsed"
      type="button"
      class="atl-gbtn"
      title="Expand sidebar"
      aria-label="Expand sidebar"
      @click="ui.toggleSidebar()"
    >
      <Icon name="panel-left" :size="14" />
    </button>

    <Crumb :parts="breadcrumbs" />

    <div style="flex: 1;" />

    <button
      v-if="shareLabel !== ''"
      type="button"
      class="atl-gbtn"
      title="Share"
      aria-label="Share"
      @click="openShare"
    >
      <Icon name="user" :size="14" />
    </button>

    <div
      v-if="shareLabel !== '' && $slots.default"
      aria-hidden="true"
      style="width: 1px; height: 18px; background: var(--c-border);"
    />

    <slot />
  </div>
</template>
