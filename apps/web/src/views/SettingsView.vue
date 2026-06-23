<script setup lang="ts">
import { computed, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import AboutPanel from '@/components/settings/AboutPanel.vue';
import AccountPanel from '@/components/settings/AccountPanel.vue';
import AdminWorkspacesPanel from '@/components/settings/AdminWorkspacesPanel.vue';
import ApiKeysPanel from '@/components/settings/ApiKeysPanel.vue';
import MembersPanel from '@/components/settings/MembersPanel.vue';
import ProjectsPanel from '@/components/settings/ProjectsPanel.vue';
import StatusesPanel from '@/components/settings/StatusesPanel.vue';
import StatusTemplatesPanel from '@/components/settings/StatusTemplatesPanel.vue';
import TagsPanel from '@/components/settings/TagsPanel.vue';
import UsersPanel from '@/components/settings/UsersPanel.vue';
import WorkspaceGeneralPanel from '@/components/settings/WorkspaceGeneralPanel.vue';
import Icon from '@/components/ui/Icon.vue';
import { useAuthStore } from '@/stores/auth';
import AppShell from '@/views/AppShell.vue';

// Section slugs are the contract between the URL (/settings/:section) and the
// panels. Adding a section means extending this union, adding one nav entry to
// the right group below, and adding one panel branch in the template.
export type SettingsSection =
  | 'account'
  | 'keys'
  | 'general'
  | 'statuses'
  | 'default-statuses'
  | 'tags'
  | 'projects'
  | 'members'
  | 'users'
  | 'workspaces'
  | 'about';

const DEFAULT_SECTION: SettingsSection = 'account';

const route = useRoute();
const router = useRouter();
const auth = useAuthStore();

interface NavEntry {
  section: SettingsSection;
  icon: string;
  label: string;
  // Root-only entries are hidden for non-root users and resolve to the default
  // section if reached directly via the URL.
  rootOnly?: boolean;
}

interface NavGroup {
  label: string;
  entries: NavEntry[];
}

const isRoot = computed(() => auth.user?.is_root === true);
const isAdmin = computed(() => isRoot.value || auth.user?.is_system_admin === true);

// Nav structure. Adding a future WORKSPACE group (general/statuses/tags) or a
// workspaces entry under ADMINISTRATION is a one-liner here plus the matching
// panel branch in the template — kept deliberately declarative for F-PANELS.
const navGroups = computed<NavGroup[]>(() => {
  const groups: NavGroup[] = [
    {
      label: 'Account',
      entries: [
        { section: 'account', icon: 'user', label: 'Account' },
        { section: 'keys', icon: 'key', label: 'API keys' },
      ],
    },
    {
      label: 'Workspace',
      entries: [
        { section: 'general', icon: 'settings', label: 'General' },
        { section: 'statuses', icon: 'kanban', label: 'Statuses' },
        { section: 'default-statuses', icon: 'kanban', label: 'Default statuses' },
        { section: 'tags', icon: 'tag', label: 'Tags' },
        { section: 'projects', icon: 'folder', label: 'Projects' },
        { section: 'members', icon: 'users', label: 'Members' },
      ],
    },
  ];

  if (isAdmin.value) {
    groups.push({
      label: 'Administration',
      entries: [
        { section: 'users', icon: 'users', label: 'Users', rootOnly: true },
        { section: 'workspaces', icon: 'layers', label: 'Workspaces', rootOnly: true },
        { section: 'about', icon: 'info', label: 'About', rootOnly: true },
      ],
    });
  }

  return groups;
});

const visibleSections = computed<Set<SettingsSection>>(
  () => new Set(navGroups.value.flatMap((group) => group.entries.map((entry) => entry.section))),
);

const rawSection = computed(() => {
  const value = route.params.section;
  return typeof value === 'string' ? value : '';
});

// Resolve the URL section to a section the current user is allowed to see.
// Unknown, missing, or root-only-for-a-non-root section -> default (account).
const activeSection = computed<SettingsSection>(() => {
  const candidate = rawSection.value as SettingsSection;
  return visibleSections.value.has(candidate) ? candidate : DEFAULT_SECTION;
});

function selectSection(section: SettingsSection): void {
  router.push({ name: 'settings', params: { section } });
}

// Keep the URL honest: a missing or unresolved section is normalised to the
// section actually rendered, so /settings and /settings/<unknown> land on
// /settings/account without leaving a stale slug in the address bar.
watch(
  [rawSection, activeSection],
  ([raw, active]) => {
    if (raw !== active) router.replace({ name: 'settings', params: { section: active } });
  },
  { immediate: true },
);
</script>

<template>
  <AppShell sidebar-title="Settings" sidebar-icon="settings" mobile-detail>
    <template #sidebar>
      <nav class="atl-settings-nav" aria-label="Settings sections">
        <div v-for="group in navGroups" :key="group.label" class="atl-settings-group">
          <div class="atl-settings-group-label">{{ group.label }}</div>
          <button
            v-for="entry in group.entries"
            :key="entry.section"
            type="button"
            class="atl-navitem"
            :class="{ on: activeSection === entry.section }"
            :data-settings-row="entry.section"
            @click="selectSection(entry.section)"
          >
            <Icon
              :name="entry.icon"
              :size="15"
              :style="{
                color: activeSection === entry.section ? 'var(--c-primary)' : 'var(--c-muted)',
                flex: '0 0 auto',
              }"
            />
            <span style="flex: 1; text-align: left;">{{ entry.label }}</span>
          </button>
        </div>
      </nav>
    </template>

    <div class="atl-settings-content">
      <AccountPanel v-if="activeSection === 'account'" />
      <ApiKeysPanel v-else-if="activeSection === 'keys'" />
      <WorkspaceGeneralPanel v-else-if="activeSection === 'general'" />
      <StatusesPanel v-else-if="activeSection === 'statuses'" />
      <StatusTemplatesPanel v-else-if="activeSection === 'default-statuses'" />
      <TagsPanel v-else-if="activeSection === 'tags'" />
      <ProjectsPanel v-else-if="activeSection === 'projects'" />
      <MembersPanel v-else-if="activeSection === 'members'" />
      <UsersPanel v-else-if="activeSection === 'users'" />
      <AdminWorkspacesPanel v-else-if="activeSection === 'workspaces'" />
      <AboutPanel v-else-if="activeSection === 'about'" />
    </div>
  </AppShell>
</template>

<style scoped>
.atl-settings-nav {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 8px;
}

.atl-settings-group {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.atl-settings-group-label {
  padding: 4px 10px;
  font-size: var(--fs-xs);
  font-weight: var(--fw-semibold);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--c-muted);
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

.atl-navitem:hover {
  background: var(--c-raised);
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
</style>
