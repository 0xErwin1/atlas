<script setup lang="ts">
import { computed, ref } from 'vue';
import { useRouter } from 'vue-router';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import { useWorkspaceSwitch } from '@/composables/useWorkspaceSwitch';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

// The active-workspace dropdown shown in the sidebar header. Switching runs the
// shared switch-and-restore flow; creating a workspace lands on its notes root.
const router = useRouter();
const ui = useUiStore();
const workspace = useWorkspaceStore();
const { switchTo } = useWorkspaceSwitch();

const activeWorkspace = computed(() =>
  workspace.workspaces.find((w) => w.slug === workspace.activeWorkspaceSlug),
);
const label = computed(() => activeWorkspace.value?.name ?? workspace.activeWorkspaceSlug ?? 'Atlas');

const newWorkspaceOpen = ref(false);

function pickWorkspace(slug: string): void {
  void switchTo(slug);
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
  <Popover placement="bottom-start" width="220px">
    <template #trigger="{ open, toggle }">
      <button
        type="button"
        class="atl-ws-switch"
        aria-label="Switch workspace"
        aria-haspopup="menu"
        :aria-expanded="open"
        @click="toggle"
      >
        <span class="atl-ws-switch-name truncate">{{ label }}</span>
        <Icon
          name="chevron-down"
          :size="14"
          :style="{
            flex: '0 0 auto',
            transform: open ? 'rotate(180deg)' : 'none',
            transition: 'transform 0.1s',
          }"
        />
      </button>
    </template>

    <template #default="{ close }">
      <div class="atl-ws-menu">
        <div class="atl-ws-menu-label">Workspaces</div>
        <div class="atl-ws-menu-sep" aria-hidden="true" />
        <button
          v-for="w in workspace.workspaces"
          :key="w.slug"
          type="button"
          role="menuitem"
          class="atl-ws-item"
          :class="{ on: w.slug === workspace.activeWorkspaceSlug }"
          @click="pickWorkspace(w.slug), close()"
        >
          <Icon :name="w.slug === workspace.activeWorkspaceSlug ? 'check' : 'folder'" :size="14" />
          {{ w.name }}
        </button>
        <div class="atl-ws-menu-sep" aria-hidden="true" />
        <button type="button" role="menuitem" class="atl-ws-item" @click="startNewWorkspace(), close()">
          <Icon name="plus" :size="14" />
          New workspace
        </button>
      </div>
    </template>
  </Popover>

  <PromptDialog
    :open="newWorkspaceOpen"
    title="New workspace"
    placeholder="Workspace name…"
    confirm-label="Create"
    @confirm="confirmNewWorkspace"
    @cancel="newWorkspaceOpen = false"
  />
</template>

<style scoped>
.atl-ws-switch {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  max-width: 100%;
  height: 26px;
  padding: 0 6px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-foreground);
  cursor: pointer;
}

.atl-ws-switch:hover {
  background: var(--c-raised);
}

.atl-ws-switch-name {
  font-size: var(--fs-base);
  font-weight: var(--fw-bold);
}

.atl-ws-menu {
  min-width: 200px;
  padding: 5px;
}

.atl-ws-menu-label {
  padding: 6px 8px 7px;
  font-size: var(--fs-sm);
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-ws-menu-sep {
  height: 1px;
  margin: 4px 0;
  background: var(--c-border);
}

.atl-ws-item {
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

.atl-ws-item:hover {
  background: var(--c-raised);
}

.atl-ws-item.on {
  color: var(--c-primary);
  font-weight: var(--fw-semibold);
}
</style>
