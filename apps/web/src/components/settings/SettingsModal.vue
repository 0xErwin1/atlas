<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import AboutPanel from '@/components/settings/AboutPanel.vue';
import AccountPanel from '@/components/settings/AccountPanel.vue';
import ApiKeysPanel from '@/components/settings/ApiKeysPanel.vue';
import UsersPanel from '@/components/settings/UsersPanel.vue';
import Icon from '@/components/ui/Icon.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import { useAuthStore } from '@/stores/auth';
import { type SettingsTab, useUiStore } from '@/stores/ui';

const ui = useUiStore();
const auth = useAuthStore();
const { isMobile } = useBreakpoint();

interface NavItem {
  tab: SettingsTab;
  icon: string;
  label: string;
}

// Users is root-only; the other tabs are available to every signed-in user.
const navItems = computed<NavItem[]>(() => {
  const items: NavItem[] = [
    { tab: 'account', icon: 'user', label: 'Account' },
    { tab: 'keys', icon: 'key', label: 'API keys' },
  ];
  if (auth.user?.is_root === true) items.push({ tab: 'users', icon: 'users', label: 'Users' });
  items.push({ tab: 'about', icon: 'info', label: 'About' });
  return items;
});

// On mobile the two-panel layout flattens to a list -> detail flow; this tracks
// whether a section has been opened (detail) versus showing the list.
const mobileDrilled = ref(false);

const currentLabel = computed(
  () => navItems.value.find((item) => item.tab === ui.settingsTab)?.label ?? 'Settings',
);

function openSection(tab: SettingsTab): void {
  ui.setSettingsTab(tab);
  mobileDrilled.value = true;
}

function backToList(): void {
  mobileDrilled.value = false;
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key !== 'Escape') return;
  // On mobile, Escape backs out of a section to the list before closing.
  if (isMobile.value && mobileDrilled.value) backToList();
  else ui.closeSettings();
}

watch(
  () => ui.settingsOpen,
  (open) => {
    if (open) {
      mobileDrilled.value = false;
      window.addEventListener('keydown', onKeydown);
    } else {
      window.removeEventListener('keydown', onKeydown);
    }
  },
);

onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown));
</script>

<template>
  <div
    v-if="ui.settingsOpen"
    class="atl-settings-overlay"
    :class="{ 'atl-settings-overlay--mobile': isMobile }"
    @click.self="ui.closeSettings()"
  >
    <div
      class="atl-settings-modal"
      :class="{ 'atl-settings-modal--mobile': isMobile }"
      role="dialog"
      aria-label="Settings"
    >
      <div class="atl-settings-header">
        <template v-if="isMobile && mobileDrilled">
          <button
            type="button"
            class="atl-settings-x"
            data-action="settings-back"
            aria-label="Back to settings"
            @click="backToList"
          >
            <Icon name="chevron-left" :size="18" />
          </button>
          <span style="font-size: 15px; font-weight: var(--fw-bold); color: var(--c-foreground); flex: 1;">
            {{ currentLabel }}
          </span>
        </template>
        <template v-else>
          <Icon name="settings" :size="16" style="color: var(--c-foreground); flex: 0 0 auto;" />
          <span style="font-size: 15px; font-weight: var(--fw-bold); color: var(--c-foreground); flex: 1;">
            Settings
          </span>
        </template>
        <button
          type="button"
          class="atl-settings-x"
          aria-label="Close settings"
          @click="ui.closeSettings()"
        >
          <Icon name="x" :size="16" />
        </button>
      </div>

      <div class="atl-settings-body">
        <nav v-if="!isMobile" class="atl-settings-nav">
          <button
            v-for="item in navItems"
            :key="item.tab"
            type="button"
            class="atl-navitem"
            :class="{ on: ui.settingsTab === item.tab }"
            @click="ui.setSettingsTab(item.tab)"
          >
            <Icon
              :name="item.icon"
              :size="15"
              :style="{ color: ui.settingsTab === item.tab ? 'var(--c-primary)' : 'var(--c-muted)', flex: '0 0 auto' }"
            />
            <span style="flex: 1; text-align: left;">{{ item.label }}</span>
          </button>
        </nav>

        <div v-else-if="!mobileDrilled" class="atl-settings-list">
          <button
            v-for="item in navItems"
            :key="item.tab"
            type="button"
            class="atl-list-row"
            :data-settings-row="item.tab"
            @click="openSection(item.tab)"
          >
            <Icon :name="item.icon" :size="17" :style="{ color: 'var(--c-muted)', flex: '0 0 auto' }" />
            <span style="flex: 1; text-align: left;">{{ item.label }}</span>
            <Icon name="chevron-right" :size="15" :style="{ color: 'var(--c-muted)', flex: '0 0 auto' }" />
          </button>
        </div>

        <div v-if="!isMobile || mobileDrilled" class="atl-settings-content">
          <AccountPanel v-if="ui.settingsTab === 'account'" />
          <ApiKeysPanel v-else-if="ui.settingsTab === 'keys'" />
          <UsersPanel v-else-if="ui.settingsTab === 'users'" />
          <AboutPanel v-else-if="ui.settingsTab === 'about'" />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.atl-settings-overlay {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: var(--c-overlay);
}

.atl-settings-modal {
  width: 864px;
  max-width: 100%;
  height: 600px;
  max-height: 100%;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--c-panel);
  border: 1px solid var(--c-border);
  border-radius: 4px;
  box-shadow: var(--shadow-lg);
}

.atl-settings-header {
  display: flex;
  align-items: center;
  gap: 9px;
  height: 48px;
  flex: 0 0 48px;
  padding: 0 10px 0 18px;
  border-bottom: 1px solid var(--c-border);
}

.atl-settings-x {
  width: 26px;
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
}

.atl-settings-x:hover {
  background: var(--c-raised);
  color: var(--c-foreground);
}

.atl-settings-body {
  flex: 1;
  display: flex;
  min-height: 0;
}

.atl-settings-nav {
  width: 196px;
  flex: 0 0 196px;
  border-right: 1px solid var(--c-border);
  background: var(--c-background);
  padding: 8px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.atl-navitem {
  display: flex;
  align-items: center;
  gap: 9px;
  height: 30px;
  padding: 0 10px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  cursor: pointer;
  font-size: 13px;
  font-weight: var(--fw-medium);
  color: var(--c-muted);
}

.atl-navitem.on {
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
  background: var(--c-selection);
  box-shadow: inset 2px 0 0 var(--c-primary);
}

.atl-settings-content {
  flex: 1;
  min-width: 0;
  overflow: auto;
  padding: 20px 24px;
}

.atl-settings-overlay--mobile {
  padding: 0;
}

.atl-settings-modal--mobile {
  width: 100%;
  height: 100%;
  max-height: 100%;
  border: none;
  border-radius: 0;
}

.atl-settings-list {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 8px;
  overflow-y: auto;
}

.atl-list-row {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  height: 48px;
  padding: 0 12px;
  border: none;
  border-radius: var(--r-md);
  background: transparent;
  cursor: pointer;
  font-size: var(--fs-lg);
  font-weight: var(--fw-medium);
  color: var(--c-foreground);
}

.atl-list-row:active {
  background: var(--c-raised);
}
</style>
