<script setup lang="ts">
import { computed, ref } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { runHardRefresh } from '@/cache/cacheRuntime';
import Avatar from '@/components/ui/Avatar.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import { useAuthStore } from '@/stores/auth';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const route = useRoute();
const router = useRouter();
const auth = useAuthStore();
const ui = useUiStore();
const workspace = useWorkspaceStore();

interface RailItem {
  name: string;
  icon: string;
  routeName: string;
  // Route names that also light up this entry — the unified Acta entry owns both
  // the notes routes and every kept tasks route under one rail section.
  activeRoutes?: string[];
}

const items: RailItem[] = [
  {
    name: 'Acta',
    icon: 'files',
    routeName: 'notes',
    activeRoutes: ['notes', 'tasks', 'task-view', 'task-detail'],
  },
];

function isActive(item: RailItem) {
  const activeRoutes = item.activeRoutes ?? [item.routeName];
  return typeof route.name === 'string' && activeRoutes.includes(route.name);
}

function navigate(item: RailItem) {
  router.push({ name: item.routeName });
}

function openSettings() {
  router.push({ name: 'settings', params: { section: 'account' } });
}

async function handleLogout() {
  await auth.logout();
  router.push({ name: 'login' });
}

const userInitials = computed(() => {
  const name = auth.user?.username ?? '?';
  return name.slice(0, 2).toUpperCase();
});

const activeWorkspace = computed(() =>
  workspace.workspaces.find((w) => w.slug === workspace.activeWorkspaceSlug),
);

const hardRefreshOpen = ref(false);
const hardRefreshPending = ref(false);

function requestHardRefresh(): void {
  hardRefreshOpen.value = true;
}

async function confirmHardRefresh(): Promise<void> {
  if (hardRefreshPending.value) return;

  const workspaceId = activeWorkspace.value?.id;
  if (workspaceId === undefined) return;

  hardRefreshPending.value = true;
  try {
    const refreshed = await runHardRefresh(workspaceId, async () => {
      router.go(0);
    });
    if (!refreshed) {
      ui.showBanner('Could not refresh cached data. Try again.', 'error');
      return;
    }

    hardRefreshOpen.value = false;
  } catch {
    ui.showBanner('Could not refresh cached data. Try again.', 'error');
  } finally {
    hardRefreshPending.value = false;
  }
}
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
      <Icon name="atlas-glyph" :size="22" :stroke-width="1.9" />
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
        @click="openSettings()"
      >
        <Icon name="settings" :size="18" />
      </button>

      <Popover placement="right-end">
        <template #trigger="{ open, toggle }">
          <button
            type="button"
            title="Account"
            aria-label="Account"
            :aria-expanded="open"
            style="border: none; background: transparent; padding: 0; cursor: pointer; display: block;"
            @click="toggle"
          >
            <Avatar :name="userInitials" :size="26" :agent="auth.apiKeyWarning" />
          </button>
        </template>

        <template #default="{ close }">
          <div class="atl-account-content">
            <div class="atl-account-id">
              <div class="atl-account-name">{{ auth.user?.username ?? 'Account' }}</div>
              <div class="atl-account-sub">Signed in</div>
            </div>
            <div class="atl-account-sep" aria-hidden="true" />
            <button type="button" role="menuitem" class="atl-account-item" @click="openSettings(), close()">
              <Icon name="settings" :size="14" />
              Settings
            </button>
            <button type="button" role="menuitem" class="atl-account-item" data-action="hard-refresh" @click="requestHardRefresh(), close()">
              <Icon name="refresh-cw" :size="14" />
              Refresh data
            </button>
            <button type="button" role="menuitem" class="atl-account-item danger" @click="handleLogout(), close()">
              <Icon name="log-out" :size="14" />
              Log out
            </button>
          </div>
        </template>
      </Popover>
    </div>

    <ConfirmDialog
      :open="hardRefreshOpen"
      title="Refresh cached data?"
      message="Saved data for this workspace will be removed before the current route reloads."
      confirm-label="Refresh data"
      confirm-icon="refresh-cw"
      tone="warning"
      @cancel="hardRefreshOpen = false"
      @confirm="confirmHardRefresh"
    />
  </nav>
</template>

<style scoped>
.atl-account-content {
  min-width: 180px;
  padding: 5px;
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

.atl-account-item.on {
  color: var(--c-primary);
  font-weight: var(--fw-semibold);
}

.atl-account-item.danger {
  color: var(--c-danger, #f07178);
}
</style>
