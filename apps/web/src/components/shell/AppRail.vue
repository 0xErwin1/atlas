<script setup lang="ts">
import { computed, ref } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { runHardRefresh } from '@/cache/cacheRuntime';
import Avatar from '@/components/ui/Avatar.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import { useWorkspaceSwitch } from '@/composables/useWorkspaceSwitch';
import { useAuthStore } from '@/stores/auth';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const route = useRoute();
const router = useRouter();
const auth = useAuthStore();
const ui = useUiStore();
const workspace = useWorkspaceStore();
const { switchTo } = useWorkspaceSwitch();

interface RailItem {
  name: string;
  icon: string;
  routeName: string;
}

const items: RailItem[] = [
  { name: 'Notes', icon: 'notes', routeName: 'notes' },
  { name: 'Tasks', icon: 'tasks', routeName: 'tasks' },
  { name: 'Search', icon: 'search', routeName: 'search' },
];

function isActive(item: RailItem) {
  return route.name === item.routeName;
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

const workspaceInitial = computed(() => {
  const label = activeWorkspace.value?.name ?? workspace.activeWorkspaceSlug ?? '';
  return label.length > 0 ? label.charAt(0).toUpperCase() : 'A';
});

const newWorkspaceOpen = ref(false);
const hardRefreshOpen = ref(false);

function requestHardRefresh(): void {
  hardRefreshOpen.value = true;
}

async function confirmHardRefresh(): Promise<void> {
  const workspaceId = activeWorkspace.value?.id;
  if (workspaceId === undefined) return;

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
  }
}

function pickWorkspace(slug: string): Promise<void> {
  return switchTo(slug);
}

function startNewWorkspace(): void {
  newWorkspaceOpen.value = true;
}

async function confirmNewWorkspace(name: string): Promise<void> {
  newWorkspaceOpen.value = false;
  const trimmed = name.trim();
  if (trimmed === '') return;

  const slug = await workspace.createWorkspace(trimmed);
  if (slug !== null) {
    router.push({ name: 'notes' });
  } else if (workspace.error !== null) {
    ui.showBanner(workspace.error, 'error');
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
      <Popover placement="right-end">
        <template #trigger="{ open, toggle }">
          <button
            type="button"
            class="atl-ws flex items-center justify-center"
            :title="`Workspace: ${activeWorkspace?.name ?? workspace.activeWorkspaceSlug ?? 'Atlas'}`"
            aria-label="Switch workspace"
            aria-haspopup="menu"
            :aria-expanded="open"
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
              cursor: pointer;
            "
            @click="toggle"
          >
            {{ workspaceInitial }}
          </button>
        </template>

        <template #default="{ close }">
          <div class="atl-account-content">
            <div class="atl-account-id">
              <div class="atl-account-name">Workspaces</div>
            </div>
            <div class="atl-account-sep" aria-hidden="true" />
            <button
              v-for="w in workspace.workspaces"
              :key="w.slug"
              type="button"
              role="menuitem"
              class="atl-account-item"
              :class="{ on: w.slug === workspace.activeWorkspaceSlug }"
              @click="pickWorkspace(w.slug), close()"
            >
              <Icon :name="w.slug === workspace.activeWorkspaceSlug ? 'check' : 'folder'" :size="14" />
              {{ w.name }}
            </button>
            <div class="atl-account-sep" aria-hidden="true" />
            <button type="button" role="menuitem" class="atl-account-item" @click="startNewWorkspace(), close()">
              <Icon name="plus" :size="14" />
              New workspace
            </button>
          </div>
        </template>
      </Popover>

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

    <PromptDialog
      :open="newWorkspaceOpen"
      title="New workspace"
      placeholder="Workspace name…"
      confirm-label="Create"
      @confirm="confirmNewWorkspace"
      @cancel="newWorkspaceOpen = false"
    />
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
