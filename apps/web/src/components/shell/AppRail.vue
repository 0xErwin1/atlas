<script setup lang="ts">
import { computed } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import Avatar from '@/components/ui/Avatar.vue';
import Icon from '@/components/ui/Icon.vue';
import { useAuthStore } from '@/stores/auth';
import { useWorkspaceStore } from '@/stores/workspace';

const route = useRoute();
const router = useRouter();
const auth = useAuthStore();
const workspace = useWorkspaceStore();

interface RailItem {
  name: string;
  icon: string;
  routeName: string;
  disabled?: boolean;
}

const items: RailItem[] = [
  { name: 'Notes', icon: 'file-text', routeName: 'notes' },
  { name: 'Tasks', icon: 'kanban', routeName: 'tasks' },
  { name: 'Search', icon: 'search', routeName: 'search' },
  { name: 'Engram', icon: 'brain', routeName: 'notes', disabled: true },
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

const workspaceInitial = computed(() => {
  const slug = workspace.activeWorkspaceSlug ?? '';
  return slug.length > 0 ? slug.charAt(0).toUpperCase() : 'A';
});
</script>

<template>
  <nav
    class="flex flex-col items-center"
    style="
      width: 48px;
      flex: 0 0 48px;
      background-color: var(--c-panel);
      border-right: 1px solid var(--c-border);
      height: 100%;
    "
    aria-label="App navigation"
  >
    <div
      class="flex items-center justify-center"
      style="height: 44px; color: var(--c-foreground);"
      title="Atlas"
    >
      <Icon name="atlas-glyph" :size="22" />
    </div>

    <div
      aria-hidden="true"
      style="width: 24px; height: 1px; background: var(--c-border); margin-bottom: 6px;"
    />

    <div class="flex flex-col" style="width: 100%;">
      <button
        v-for="item in items"
        :key="item.name"
        type="button"
        :title="item.name"
        :disabled="item.disabled"
        :aria-label="item.name"
        :aria-current="isActive(item) ? 'page' : undefined"
        class="atl-railitem flex items-center justify-center"
        :class="{ on: isActive(item) }"
        :style="`
          width: 48px;
          height: 40px;
          border: none;
          cursor: ${item.disabled ? 'not-allowed' : 'pointer'};
          background-color: ${isActive(item) ? 'var(--c-selection)' : 'transparent'};
          box-shadow: ${isActive(item) ? 'inset 2px 0 0 var(--c-primary)' : 'none'};
          color: ${isActive(item) ? 'var(--c-primary)' : 'var(--c-muted)'};
          opacity: ${item.disabled ? '0.4' : '1'};
        `"
        @click="navigate(item)"
      >
        <Icon :name="item.icon" :size="20" :stroke-width="isActive(item) ? 2 : 1.8" />
      </button>

      <button
        type="button"
        title="Add app"
        aria-label="Add app"
        disabled
        class="atl-railitem flex items-center justify-center"
        style="
          width: 48px;
          height: 36px;
          border: none;
          background: transparent;
          color: var(--c-muted);
          cursor: not-allowed;
          opacity: 0.4;
        "
      >
        <Icon name="plus" :size="16" />
      </button>
    </div>

    <div style="flex: 1;" />

    <div class="flex flex-col items-center" style="gap: 8px; padding-bottom: 10px;">
      <div
        class="atl-ws flex items-center justify-center"
        :title="`Workspace: ${workspace.activeWorkspaceSlug ?? 'Atlas'}`"
        style="
          width: 26px;
          height: 26px;
          border-radius: var(--r-sm);
          background: var(--c-raised);
          border: 1px solid var(--c-border);
          font-family: var(--font-mono);
          font-size: 12px;
          font-weight: var(--fw-bold);
          color: var(--c-primary);
        "
      >
        {{ workspaceInitial }}
      </div>

      <button
        type="button"
        title="Settings"
        aria-label="Settings"
        class="atl-railitem flex items-center justify-center"
        style="
          width: 40px;
          height: 32px;
          border: none;
          cursor: pointer;
          background: transparent;
          color: var(--c-muted);
        "
      >
        <Icon name="settings" :size="18" />
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
