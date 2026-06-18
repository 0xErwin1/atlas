<script setup lang="ts">
import { computed, ref } from 'vue';
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
}

const items: RailItem[] = [
  { name: 'Notes', icon: 'file-text', routeName: 'notes' },
  { name: 'Tasks', icon: 'kanban', routeName: 'tasks' },
  { name: 'Search', icon: 'search', routeName: 'search' },
];

function isActive(item: RailItem) {
  return route.name === item.routeName;
}

function navigate(item: RailItem) {
  router.push({ name: item.routeName });
}

const accountOpen = ref(false);

async function handleLogout() {
  accountOpen.value = false;
  await auth.logout();
  router.push({ name: 'login' });
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
        :aria-label="item.name"
        :aria-current="isActive(item) ? 'page' : undefined"
        class="atl-railitem flex items-center justify-center"
        :class="{ on: isActive(item) }"
        :style="`
          width: 48px;
          height: 40px;
          border: none;
          cursor: pointer;
          background-color: ${isActive(item) ? 'var(--c-selection)' : 'transparent'};
          box-shadow: ${isActive(item) ? 'inset 2px 0 0 var(--c-primary)' : 'none'};
          color: ${isActive(item) ? 'var(--c-primary)' : 'var(--c-muted)'};
        `"
        @click="navigate(item)"
      >
        <Icon :name="item.icon" :size="20" :stroke-width="isActive(item) ? 2 : 1.8" />
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

      <div style="position: relative;">
        <button
          type="button"
          title="Account"
          aria-label="Account"
          :aria-expanded="accountOpen"
          style="border: none; background: transparent; padding: 0; cursor: pointer; display: block;"
          @click="accountOpen = !accountOpen"
        >
          <Avatar :name="userInitials" :size="26" :agent="auth.apiKeyWarning" />
        </button>

        <template v-if="accountOpen">
          <div
            class="atl-account-backdrop"
            aria-hidden="true"
            @click="accountOpen = false"
            @contextmenu.prevent="accountOpen = false"
          />
          <div class="atl-account-menu" role="menu">
            <div class="atl-account-id">
              <div class="atl-account-name">{{ auth.user?.username ?? 'Account' }}</div>
              <div class="atl-account-sub">Signed in</div>
            </div>
            <div class="atl-account-sep" aria-hidden="true" />
            <button type="button" role="menuitem" class="atl-account-item danger" @click="handleLogout">
              <Icon name="log-out" :size="14" />
              Log out
            </button>
          </div>
        </template>
      </div>
    </div>
  </nav>
</template>

<style scoped>
.atl-account-backdrop {
  position: fixed;
  inset: 0;
  z-index: 40;
}

.atl-account-menu {
  position: absolute;
  bottom: 0;
  left: calc(100% + 8px);
  z-index: 41;
  min-width: 180px;
  padding: 5px;
  background: var(--c-panel);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  box-shadow: var(--shadow-md, 0 8px 24px rgba(0, 0, 0, 0.35));
}

.atl-account-id {
  padding: 6px 8px 7px;
}

.atl-account-name {
  font-size: var(--fs-sm);
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-account-sub {
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-account-sep {
  height: 1px;
  margin: 4px 0;
  background: var(--c-border);
}

.atl-account-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 7px 8px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  cursor: pointer;
  font-size: var(--fs-sm);
  color: var(--c-foreground);
  text-align: left;
}

.atl-account-item:hover {
  background: var(--c-raised);
}

.atl-account-item.danger {
  color: var(--c-danger, #f07178);
}
</style>
