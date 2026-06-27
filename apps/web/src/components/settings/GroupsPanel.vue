<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import EmptyState from '@/components/states/EmptyState.vue';
import Btn from '@/components/ui/Btn.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Icon from '@/components/ui/Icon.vue';
import { initials } from '@/lib/format';
import { type GroupDto, useGroupsStore } from '@/stores/groups';
import { useUiStore } from '@/stores/ui';
import { type PrincipalDto, useWorkspaceStore } from '@/stores/workspace';

const groupsStore = useGroupsStore();
const wsStore = useWorkspaceStore();
const ui = useUiStore();

const activeWs = computed(() => wsStore.activeWorkspaceSlug ?? '');

const loading = ref(false);
const newName = ref('');
const creating = ref(false);

const expandedId = ref<string | null>(null);
const memberQuery = ref('');
const busy = ref(false);

const deleteTarget = ref<GroupDto | null>(null);

// Workspace users only — groups hold users, never api keys.
const userMembers = computed<PrincipalDto[]>(() =>
  wsStore.members.filter((m) => m.principal_type === 'user'),
);

function displayName(userId: string): string {
  return wsStore.members.find((m) => m.id === userId)?.display ?? userId;
}

const memberUserIds = computed(() => new Set(groupsStore.members.map((m) => m.user_id)));

const addMatches = computed<PrincipalDto[]>(() => {
  const q = memberQuery.value.trim().toLowerCase();
  if (q === '') return [];

  return userMembers.value.filter(
    (m) => !memberUserIds.value.has(m.id) && m.display.toLowerCase().includes(q),
  );
});

async function reload(): Promise<void> {
  const ws = activeWs.value;
  if (ws === '') return;

  loading.value = true;
  await groupsStore.load(ws);
  if (wsStore.members.length === 0) await wsStore.loadMembers(ws);
  loading.value = false;
}

onMounted(reload);

watch(activeWs, (ws, prev) => {
  if (ws !== prev) {
    expandedId.value = null;
    void reload();
  }
});

async function createGroup(): Promise<void> {
  const ws = activeWs.value;
  const name = newName.value.trim();
  if (ws === '' || name === '') return;

  creating.value = true;
  const ok = await groupsStore.create(ws, name);
  creating.value = false;

  if (ok) {
    newName.value = '';
    ui.showBanner('Group created', 'success');
  } else if (groupsStore.error) {
    ui.showBanner(groupsStore.error, 'error');
  }
}

async function toggleExpand(group: GroupDto): Promise<void> {
  const ws = activeWs.value;
  if (ws === '') return;

  if (expandedId.value === group.id) {
    expandedId.value = null;
    return;
  }

  expandedId.value = group.id;
  memberQuery.value = '';
  await groupsStore.loadMembers(ws, group.id);
}

async function addMember(member: PrincipalDto): Promise<void> {
  const ws = activeWs.value;
  const groupId = expandedId.value;
  if (ws === '' || groupId === null || busy.value) return;

  busy.value = true;
  const ok = await groupsStore.addMember(ws, groupId, member.id);
  busy.value = false;

  if (ok) {
    memberQuery.value = '';
    ui.showBanner(`${member.display} added to the group`, 'success');
  } else if (groupsStore.error) {
    ui.showBanner(groupsStore.error, 'error');
  }
}

async function removeMember(userId: string): Promise<void> {
  const ws = activeWs.value;
  const groupId = expandedId.value;
  if (ws === '' || groupId === null || busy.value) return;

  busy.value = true;
  const ok = await groupsStore.removeMember(ws, groupId, userId);
  busy.value = false;

  if (ok) ui.showBanner('Member removed', 'success');
  else if (groupsStore.error) ui.showBanner(groupsStore.error, 'error');
}

async function confirmDelete(): Promise<void> {
  const target = deleteTarget.value;
  deleteTarget.value = null;
  if (target === null) return;

  const ws = activeWs.value;
  if (ws === '') return;

  const ok = await groupsStore.remove(ws, target.id);
  if (ok) {
    if (expandedId.value === target.id) expandedId.value = null;
    ui.showBanner(`Group "${target.name}" deleted`, 'success');
  } else if (groupsStore.error) {
    ui.showBanner(groupsStore.error, 'error');
  }
}
</script>

<template>
  <div>
    <div class="atl-panel-head">
      <div class="atl-panel-title">Groups</div>
      <div class="atl-panel-sub">
        Bundle workspace members into groups, then grant a whole group access at once
      </div>
    </div>

    <form class="atl-group-create" @submit.prevent="createGroup">
      <input
        v-model="newName"
        type="text"
        data-group-name
        placeholder="New group name"
        autocomplete="off"
        class="atl-group-input"
        :disabled="creating"
      />
      <Btn type="submit" variant="primary" data-action="create-group" :disabled="creating || newName.trim() === ''">
        <Icon name="plus" :size="14" />
        Create group
      </Btn>
    </form>

    <div v-if="loading" style="font-size: 13px; color: var(--c-muted); padding: 8px;">
      Loading&hellip;
    </div>

    <EmptyState
      v-else-if="groupsStore.groups.length === 0"
      compact
      icon="users"
      title="No groups yet"
      hint="Create a group to grant several members access at once. Groups can be shared on workspaces and projects just like people."
    />

    <div v-else class="atl-groups-list">
      <div v-for="group in groupsStore.groups" :key="group.id" class="atl-group-card" data-group-row>
        <div class="atl-group-row">
          <button
            type="button"
            class="atl-group-head"
            :aria-expanded="expandedId === group.id"
            data-action="toggle-group"
            @click="toggleExpand(group)"
          >
            <span class="atl-group-glyph">
              <Icon name="users" :size="14" />
            </span>
            <span class="atl-group-name">{{ group.name }}</span>
            <Icon
              :name="expandedId === group.id ? 'chevron-down' : 'chevron-right'"
              :size="14"
              :style="{ color: 'var(--c-muted)', flex: '0 0 auto' }"
            />
          </button>

          <button
            type="button"
            class="atl-group-delete"
            data-action="delete-group"
            title="Delete group"
            @click="deleteTarget = group"
          >
            <Icon name="trash-2" :size="13" />
          </button>
        </div>

        <div v-if="expandedId === group.id" class="atl-group-members">
          <div class="atl-add-member">
            <div class="atl-add-member-input">
              <Icon name="user-plus" :size="14" :style="{ color: 'var(--c-muted)' }" />
              <input
                v-model="memberQuery"
                type="text"
                data-add-member-search
                placeholder="Add a workspace member by name"
                autocomplete="off"
              />
            </div>
            <div v-if="addMatches.length > 0" class="atl-add-member-results" role="listbox">
              <button
                v-for="m in addMatches"
                :key="m.id"
                type="button"
                role="option"
                data-add-member-option
                class="atl-add-member-option"
                :disabled="busy"
                @click="addMember(m)"
              >
                <span class="atl-mini-avatar">{{ initials(m.display) }}</span>
                <span class="flex-1 truncate">{{ m.display }}</span>
              </button>
            </div>
          </div>

          <div v-if="groupsStore.members.length === 0" class="atl-group-empty-members">
            No members in this group yet.
          </div>

          <div
            v-for="m in groupsStore.members"
            :key="m.user_id"
            class="atl-group-member-row"
            data-group-member
          >
            <span class="atl-mini-avatar">{{ initials(displayName(m.user_id)) }}</span>
            <span class="atl-group-member-name">{{ displayName(m.user_id) }}</span>
            <button
              type="button"
              class="atl-member-remove"
              data-action="remove-member"
              title="Remove from group"
              :disabled="busy"
              @click="removeMember(m.user_id)"
            >
              <Icon name="user-minus" :size="13" />
            </button>
          </div>
        </div>
      </div>
    </div>

    <div class="atl-members-note">
      <Icon name="shield" :size="13" style="color: var(--c-primary);" />
      Only owners and admins can create groups, delete them, or change their membership.
    </div>

    <ConfirmDialog
      :open="deleteTarget !== null"
      tone="danger"
      title="Delete this group?"
      message="Members lose any access they had only through this group. The members themselves are not removed from the workspace."
      :detail="deleteTarget?.name"
      detail-icon="users"
      confirm-label="Delete group"
      confirm-icon="trash-2"
      @confirm="confirmDelete"
      @cancel="deleteTarget = null"
    />
  </div>
</template>

<style scoped>
.atl-panel-head {
  margin-bottom: 16px;
}

.atl-panel-title {
  font-size: 15px;
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
}

.atl-panel-sub {
  font-size: 12px;
  color: var(--c-muted);
  margin-top: 3px;
}

.atl-group-create {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 16px;
}

.atl-group-input {
  flex: 1;
  height: 32px;
  padding: 0 11px;
  background-color: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  outline: none;
  font-size: var(--fs-base);
  color: var(--c-foreground);
}

.atl-group-input:focus {
  border-color: var(--c-primary);
}

.atl-groups-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.atl-group-card {
  border: 1px solid var(--c-border);
  border-radius: 4px;
  overflow: hidden;
}

.atl-group-row {
  display: flex;
  align-items: center;
  height: 46px;
  padding: 0 8px 0 12px;
}

.atl-group-head {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  height: 100%;
  border: none;
  background: transparent;
  cursor: pointer;
  text-align: left;
  color: var(--c-foreground);
}

.atl-group-glyph {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  flex: 0 0 auto;
  border-radius: 2px;
  color: var(--c-primary);
  background: color-mix(in srgb, var(--c-primary) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--c-primary) 40%, transparent);
}

.atl-group-name {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-group-delete {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 24px;
  flex: 0 0 auto;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-danger);
  cursor: pointer;
}

.atl-group-delete:hover {
  background: var(--c-raised);
}

.atl-group-members {
  border-top: 1px solid var(--c-border);
  padding: 12px;
  background: var(--c-raised);
}

.atl-add-member {
  position: relative;
  margin-bottom: 10px;
}

.atl-add-member-input {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 32px;
  padding: 0 10px;
  background-color: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
}

.atl-add-member-input input {
  flex: 1;
  min-width: 0;
  height: 100%;
  border: none;
  outline: none;
  background: transparent;
  font-size: var(--fs-base);
  color: var(--c-foreground);
}

.atl-add-member-results {
  position: absolute;
  top: 36px;
  left: 0;
  right: 0;
  max-height: 200px;
  overflow-y: auto;
  background-color: var(--c-panel);
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg);
  box-shadow: var(--shadow-lg);
  padding: 4px;
  z-index: 10;
}

.atl-add-member-option {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  height: 34px;
  padding: 0 8px;
  border: none;
  border-radius: var(--r-md);
  background: transparent;
  cursor: pointer;
  text-align: left;
  font-size: var(--fs-base);
  color: var(--c-foreground);
}

.atl-add-member-option:hover:enabled {
  background: var(--c-raised);
}

.atl-group-member-row {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 40px;
  padding: 0 4px;
}

.atl-group-member-row + .atl-group-member-row {
  border-top: 1px solid var(--c-border);
}

.atl-group-member-name {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  font-weight: var(--fw-medium);
  color: var(--c-foreground);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-group-empty-members {
  font-size: 12.5px;
  color: var(--c-muted);
  padding: 6px 4px;
}

.atl-mini-avatar {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  flex: 0 0 auto;
  border-radius: 2px;
  background-color: var(--c-raised);
  border: 1px solid var(--c-border);
  font-family: var(--font-mono);
  font-size: 9px;
  font-weight: 700;
  color: var(--c-foreground);
}

.atl-member-remove {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 24px;
  flex: 0 0 auto;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-danger);
  cursor: pointer;
}

.atl-member-remove:hover:enabled {
  background: var(--c-panel);
}

.atl-members-note {
  display: flex;
  align-items: center;
  gap: 7px;
  margin-top: 14px;
  font-size: 12px;
  color: var(--c-muted);
}
</style>
