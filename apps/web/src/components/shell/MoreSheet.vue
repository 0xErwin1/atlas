<script setup lang="ts">
import { computed } from 'vue';
import { useRouter } from 'vue-router';
import Avatar from '@/components/ui/Avatar.vue';
import BottomSheet from '@/components/ui/BottomSheet.vue';
import Icon from '@/components/ui/Icon.vue';
import { useAuthStore } from '@/stores/auth';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

defineProps<{
  open: boolean;
}>();

const emit = defineEmits<{
  close: [];
}>();

const router = useRouter();
const auth = useAuthStore();
const ui = useUiStore();
const workspace = useWorkspaceStore();

const userInitials = computed(() => (auth.user?.username ?? '?').slice(0, 2).toUpperCase());

const workspaceSlug = computed(() => workspace.activeWorkspaceSlug ?? 'Atlas');

const workspaceInitial = computed(() => workspaceSlug.value.charAt(0).toUpperCase() || 'A');

function openSettings(): void {
  emit('close');
  ui.openSettings();
}

async function handleLogout(): Promise<void> {
  emit('close');
  await auth.logout();
  router.push({ name: 'login' });
}
</script>

<template>
  <BottomSheet :open="open" title="More" @close="emit('close')">
    <div
      class="flex items-center"
      style="gap: 11px; padding: 6px 2px 12px;"
    >
      <div
        class="flex items-center justify-center"
        style="
          width: 34px;
          height: 34px;
          border-radius: var(--r-sm);
          background: var(--c-raised);
          border: 1px solid var(--c-border);
          font-family: var(--font-mono);
          font-size: 14px;
          font-weight: var(--fw-bold);
          color: var(--c-primary);
          flex: 0 0 auto;
        "
      >
        {{ workspaceInitial }}
      </div>
      <div class="min-w-0">
        <div
          style="font-size: 10px; font-weight: var(--fw-semibold); letter-spacing: 0.06em; text-transform: uppercase; color: var(--c-muted);"
        >
          Workspace
        </div>
        <div class="truncate" style="font-size: var(--fs-base); font-weight: var(--fw-semibold); color: var(--c-foreground);">
          {{ workspaceSlug }}
        </div>
      </div>
    </div>

    <div style="height: 1px; background: var(--c-border); margin: 2px 0 10px;" aria-hidden="true" />

    <div class="flex items-center" style="gap: 11px; padding: 4px 2px 12px;">
      <Avatar :name="userInitials" :size="32" :agent="auth.apiKeyWarning" />
      <div class="min-w-0">
        <div class="truncate" style="font-size: var(--fs-base); font-weight: var(--fw-semibold); color: var(--c-foreground);">
          {{ auth.user?.username ?? 'Account' }}
        </div>
        <div style="font-size: var(--fs-xs); color: var(--c-muted);">Signed in</div>
      </div>
    </div>

    <button type="button" class="atl-more-item" @click="openSettings">
      <Icon name="settings" :size="17" />
      <span class="flex-1 text-left">Settings</span>
      <Icon name="chevron-right" :size="15" :style="{ color: 'var(--c-muted)' }" />
    </button>

    <button type="button" class="atl-more-item danger" @click="handleLogout">
      <Icon name="log-out" :size="17" />
      <span class="flex-1 text-left">Log out</span>
    </button>
  </BottomSheet>
</template>

<style scoped>
.atl-more-item {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  height: 46px;
  padding: 0 4px;
  border: none;
  border-radius: var(--r-md);
  background: transparent;
  cursor: pointer;
  font-size: var(--fs-lg);
  font-weight: var(--fw-medium);
  color: var(--c-foreground);
}

.atl-more-item:active {
  background: var(--c-raised);
}

.atl-more-item.danger {
  color: var(--c-danger, #f07178);
}
</style>
