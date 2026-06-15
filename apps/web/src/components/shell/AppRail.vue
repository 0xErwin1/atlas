<script setup lang="ts">
import { computed } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import Avatar from '@/components/ui/Avatar.vue';
import Icon from '@/components/ui/Icon.vue';
import { useAuthStore } from '@/stores/auth';

const route = useRoute();
const router = useRouter();
const auth = useAuthStore();

interface RailItem {
  name: string;
  icon: string;
  routeName: string;
  disabled?: boolean;
}

const items: RailItem[] = [
  { name: 'Notes', icon: 'file-text', routeName: 'notes' },
  { name: 'Tasks', icon: 'layout-kanban', routeName: 'tasks' },
  { name: 'Search', icon: 'search', routeName: 'search' },
  { name: 'Engram', icon: 'brain', routeName: 'notes', disabled: true },
  { name: 'New', icon: 'plus', routeName: 'notes', disabled: true },
];

function isActive(item: RailItem) {
  return !item.disabled && route.name === item.routeName;
}

function navigate(item: RailItem) {
  if (item.disabled) return;
  router.push({ name: item.routeName });
}

const userInitials = computed(() => {
  const name = auth.user?.username ?? '?';
  return name.slice(0, 2).toUpperCase();
});
</script>

<template>
  <nav
    class="flex flex-col items-center"
    style="
      width: 48px;
      background-color: var(--c-panel);
      border-right: 1px solid var(--c-border);
      flex-shrink: 0;
      height: 100%;
    "
    aria-label="App navigation"
  >
    <div
      class="flex items-center justify-center"
      style="width: 48px; height: 48px; padding: 2px;"
    >
      <Icon
        name="atlas-glyph"
        :size="28"
        style="color: var(--c-primary);"
      />
    </div>

    <div class="flex flex-col items-center flex-1 gap-0.5 py-1">
      <button
        v-for="item in items"
        :key="item.name"
        type="button"
        :title="item.name"
        :disabled="item.disabled"
        :aria-label="item.name"
        :aria-current="isActive(item) ? 'page' : undefined"
        class="flex items-center justify-center"
        :style="`
          width: 48px;
          height: 40px;
          border: none;
          cursor: ${item.disabled ? 'not-allowed' : 'pointer'};
          border-radius: var(--r-md);
          position: relative;
          background-color: ${isActive(item) ? 'var(--c-selection)' : 'transparent'};
          color: ${item.disabled ? 'var(--c-muted)' : isActive(item) ? 'var(--c-primary)' : 'var(--c-muted)'};
          opacity: ${item.disabled ? '0.4' : '1'};
        `"
        @click="navigate(item)"
      >
        <span
          v-if="isActive(item)"
          style="
            position: absolute;
            left: 0;
            top: 8px;
            bottom: 8px;
            width: 2px;
            border-radius: 0 1px 1px 0;
            background-color: var(--c-primary);
          "
          aria-hidden="true"
        />
        <Icon :name="item.icon" :size="20" />
      </button>
    </div>

    <div class="flex flex-col items-center gap-1 pb-2">
      <button
        type="button"
        title="Settings"
        aria-label="Settings"
        class="flex items-center justify-center"
        style="
          width: 32px;
          height: 32px;
          border: none;
          cursor: pointer;
          border-radius: var(--r-md);
          background: transparent;
          color: var(--c-muted);
        "
      >
        <Icon name="settings" :size="16" />
      </button>

      <Avatar
        :name="userInitials"
        :size="26"
        :agent="auth.apiKeyWarning"
        style="cursor: pointer;"
      />
    </div>
  </nav>
</template>
