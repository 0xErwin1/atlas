<script setup lang="ts">
import { ref } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import MoreSheet from '@/components/shell/MoreSheet.vue';
import Icon from '@/components/ui/Icon.vue';

interface Tab {
  id: string;
  icon: string;
  label: string;
  routeName?: string;
}

const tabs: Tab[] = [
  { id: 'notes', icon: 'file-text', label: 'Notes', routeName: 'notes' },
  { id: 'tasks', icon: 'kanban', label: 'Tasks', routeName: 'tasks' },
  { id: 'search', icon: 'search', label: 'Search', routeName: 'search' },
  { id: 'more', icon: 'more-horizontal', label: 'More' },
];

const route = useRoute();
const router = useRouter();

const moreOpen = ref(false);

function isActive(tab: Tab): boolean {
  return tab.routeName !== undefined && route.name === tab.routeName;
}

function onTab(tab: Tab): void {
  if (tab.id === 'more') {
    moreOpen.value = true;
    return;
  }
  if (tab.routeName) router.push({ name: tab.routeName });
}
</script>

<template>
  <nav
    class="flex items-stretch"
    style="
      height: 56px;
      flex: 0 0 56px;
      background-color: var(--c-panel);
      border-top: 1px solid var(--c-border);
      padding-bottom: env(safe-area-inset-bottom, 0);
    "
    aria-label="App navigation"
  >
    <button
      v-for="tab in tabs"
      :key="tab.id"
      type="button"
      :data-tab="tab.id"
      :aria-label="tab.label"
      :aria-current="isActive(tab) ? 'page' : undefined"
      class="flex flex-col items-center justify-center flex-1"
      :style="`
        gap: 3px;
        border: none;
        background: transparent;
        cursor: pointer;
        color: ${isActive(tab) ? 'var(--c-primary)' : 'var(--c-muted)'};
      `"
      @click="onTab(tab)"
    >
      <Icon :name="tab.icon" :size="21" :stroke-width="isActive(tab) ? 2 : 1.8" />
      <span style="font-size: 10px; font-weight: var(--fw-medium);">{{ tab.label }}</span>
    </button>
  </nav>

  <MoreSheet :open="moreOpen" @close="moreOpen = false" />
</template>
