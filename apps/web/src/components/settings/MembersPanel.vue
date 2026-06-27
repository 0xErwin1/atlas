<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import EmptyState from '@/components/states/EmptyState.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import Btn from '@/components/ui/Btn.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import Icon from '@/components/ui/Icon.vue';
import { initials } from '@/lib/format';
import {
  coerceWorkspaceRole,
  workspaceRoleLabel as roleLabel,
  workspaceRoleTagClass as roleTagClass,
  type WorkspaceRole,
  workspaceRoleOptions,
} from '@/lib/workspaceRoles';
import { useAuthStore } from '@/stores/auth';
import { useUiStore } from '@/stores/ui';
import { type PrincipalDto, type UserDto, useWorkspaceStore } from '@/stores/workspace';

const wsStore = useWorkspaceStore();
const auth = useAuthStore();
const ui = useUiStore();

type Role = WorkspaceRole;
const roleOptions: DropdownOption[] = workspaceRoleOptions({ includeOwner: true });

const loading = ref(false);
const busyId = ref<string | null>(null);
const removeTarget = ref<PrincipalDto | null>(null);

// Human members carry a role and the full management controls.
const userMembers = computed<PrincipalDto[]>(() =>
  wsStore.members.filter((m) => m.principal_type === 'user'),
);

// Agents (api-key principals) carry no role; they are listed in a separate
// section with their key type and never get role/remove controls here.
const agentMembers = computed<PrincipalDto[]>(() =>
  wsStore.members.filter((m) => m.principal_type === 'api_key'),
);

const activeWs = computed(() => wsStore.activeWorkspaceSlug ?? '');

function roleOf(m: PrincipalDto): Role {
  return coerceWorkspaceRole(m.role);
}

function isSelf(m: PrincipalDto): boolean {
  return auth.user?.id != null && m.id === auth.user.id;
}

const isBreakGlass = computed(() => auth.user?.is_root === true || auth.user?.is_system_admin === true);

// The caller's own workspace role is not on the auth user — it is read from the
// member list entry matching the signed-in user. Break-glass callers may have
// no membership in this workspace at all.
const callerRole = computed<Role | null>(() => {
  const id = auth.user?.id;
  if (id == null) return null;
  const self = wsStore.members.find((m) => m.id === id);
  return self !== undefined ? roleOf(self) : null;
});

// UX-only gate: the backend is the source of truth, but the panel avoids
// offering dead-end actions. Owners and admins (or break-glass) may mutate.
const canManage = computed(
  () => isBreakGlass.value || callerRole.value === 'owner' || callerRole.value === 'admin',
);

const ownerCount = computed(() => userMembers.value.filter((m) => roleOf(m) === 'owner').length);

type AccountStatus = 'deactivated' | 'pending';

// Only deactivated/pending members are badged. `active` and a missing
// account_status (older payloads) render no status badge.
function statusOf(m: PrincipalDto): AccountStatus | null {
  if (m.account_status === 'deactivated') return 'deactivated';
  if (m.account_status === 'pending') return 'pending';
  return null;
}

function statusLabel(status: AccountStatus): string {
  return status === 'deactivated' ? 'Deactivated' : 'Pending';
}

function statusTagClass(status: AccountStatus): string {
  return status === 'deactivated' ? 'atl-status-deactivated' : 'atl-status-pending';
}

function agentBadgeLabel(m: PrincipalDto): string {
  return (m.key_type ?? 'agent').toUpperCase();
}

// Demoting or removing the sole remaining owner always fails server-side
// (urn:atlas:error:last-owner). Disable those controls so the UI never offers
// an action that is guaranteed to 409.
function isLastOwner(m: PrincipalDto): boolean {
  return roleOf(m) === 'owner' && ownerCount.value <= 1;
}

function roleSelectDisabled(m: PrincipalDto): boolean {
  if (busyId.value !== null) return true;
  return isLastOwner(m);
}

function removeDisabled(m: PrincipalDto): boolean {
  if (busyId.value !== null) return true;
  return isLastOwner(m);
}

function removeTitle(m: PrincipalDto): string {
  if (isLastOwner(m)) return 'A workspace must keep at least one owner';
  return 'Remove from workspace';
}

const removeDetail = computed(() => {
  const target = removeTarget.value;
  if (target === null) return undefined;
  return target.display;
});

async function reload(): Promise<void> {
  const ws = activeWs.value;
  if (ws === '') return;

  loading.value = true;
  await wsStore.loadMembers(ws);
  loading.value = false;
}

onMounted(reload);

watch(activeWs, (ws, prev) => {
  if (ws !== prev) void reload();
});

async function changeRole(m: PrincipalDto, next: Role): Promise<void> {
  if (next === roleOf(m)) return;

  const ws = activeWs.value;
  if (ws === '') return;

  busyId.value = m.id;
  const ok = await wsStore.updateMemberRole(ws, m.id, next);
  busyId.value = null;

  if (ok) {
    ui.showBanner(`${m.display} is now ${roleLabel(next).toLowerCase()}`, 'success');
  } else if (wsStore.error) {
    ui.showBanner(wsStore.error, 'error');
    await reload();
  }
}

async function confirmRemove(): Promise<void> {
  const target = removeTarget.value;
  removeTarget.value = null;
  if (target === null) return;

  const ws = activeWs.value;
  if (ws === '') return;

  busyId.value = target.id;
  const ok = await wsStore.removeMember(ws, target.id);
  busyId.value = null;

  if (ok) ui.showBanner(`${target.display} removed from the workspace`, 'success');
  else if (wsStore.error) ui.showBanner(wsStore.error, 'error');
}

const addOpen = ref(false);
const addLoading = ref(false);
const addSubmitting = ref(false);
const selectedUserId = ref<string | null>(null);
const selectedRole = ref<Role>('member');
const addError = ref<string | null>(null);

const assignableUsers = computed<UserDto[]>(() => wsStore.assignableUsers);

// Only an owner (or a break-glass global admin) may grant the owner role; the
// backend returns 403 otherwise, so admins never see the option.
const canGrantOwner = computed(() => isBreakGlass.value || wsStore.myWorkspaceRole === 'owner');

const addableRoles = computed<DropdownOption[]>(() =>
  roleOptions.filter((r) => r.value !== 'owner' || canGrantOwner.value),
);

function userIsPending(u: UserDto): boolean {
  return u.activated_at === null || u.activated_at === undefined;
}

async function openAdd(): Promise<void> {
  const ws = activeWs.value;
  if (ws === '') return;

  addOpen.value = true;
  addError.value = null;
  selectedUserId.value = null;
  selectedRole.value = 'member';

  addLoading.value = true;
  await wsStore.loadAssignableUsers(ws);
  addLoading.value = false;
}

function closeAdd(): void {
  if (addSubmitting.value) return;
  addOpen.value = false;
}

async function confirmAdd(): Promise<void> {
  if (addSubmitting.value) return;

  const ws = activeWs.value;
  const userId = selectedUserId.value;
  if (ws === '' || userId === null) return;

  addSubmitting.value = true;
  addError.value = null;
  const ok = await wsStore.addMember(ws, userId, selectedRole.value);

  if (ok) {
    await reload();
    addSubmitting.value = false;
    addOpen.value = false;
    ui.showBanner('Member added to the workspace', 'success');
    return;
  }

  addSubmitting.value = false;
  addError.value = wsStore.error ?? 'Failed to add member';
}
</script>

<template>
  <div>
    <div class="atl-panel-head">
      <div style="flex: 1; min-width: 0;">
        <div class="atl-panel-title">Members</div>
        <div class="atl-panel-sub">Manage who belongs to this workspace and what they can do</div>
      </div>
      <Btn
        v-if="canManage"
        variant="primary"
        data-action="add-member"
        @click="openAdd"
      >
        <Icon name="user-plus" :size="14" />
        Add member
      </Btn>
    </div>

    <div v-if="loading" style="font-size: 13px; color: var(--c-muted); padding: 8px;">
      Loading&hellip;
    </div>

    <template v-else>
      <div class="atl-members-section-label">Members</div>

      <EmptyState
        v-if="userMembers.length === 0"
        compact
        icon="users"
        title="No members yet"
        hint="Workspace members appear here with their role. Owners and admins can change roles or remove members."
      />

      <div v-else class="atl-members-table" data-section="members">
        <div class="atl-members-head">
          <div style="flex: 2;">Member</div>
          <div style="flex: 0 0 120px;">Role</div>
          <div style="flex: 0 0 150px;"></div>
        </div>

        <div v-for="m in userMembers" :key="m.id" class="atl-members-row" data-member-row>
          <div style="flex: 2; display: flex; align-items: center; gap: 10px; min-width: 0;">
            <Avatar :name="initials(m.display)" :size="26" />
            <div class="flex items-center" style="gap: 6px; min-width: 0;">
              <span class="atl-member-name">{{ m.display }}</span>
              <span v-if="isSelf(m)" class="atl-you">you</span>
            </div>
          </div>

          <div style="flex: 0 0 120px; display: flex; align-items: center; gap: 6px; flex-wrap: wrap;">
            <span class="atl-role-badge" :class="roleTagClass(roleOf(m))">{{ roleLabel(roleOf(m)) }}</span>
            <span
              v-if="statusOf(m) !== null"
              class="atl-role-badge atl-status-badge"
              :class="statusTagClass(statusOf(m)!)"
              :title="statusOf(m) === 'deactivated' ? 'This account is deactivated' : 'Pending activation'"
            >{{ statusLabel(statusOf(m)!) }}</span>
          </div>

          <div style="flex: 0 0 150px; display: flex; justify-content: flex-end; align-items: center; gap: 6px;">
            <template v-if="canManage">
              <Dropdown
                :options="roleOptions"
                :model-value="roleOf(m)"
                :disabled="roleSelectDisabled(m)"
                @change="(v) => changeRole(m, v as Role)"
              />
              <button
                type="button"
                class="atl-member-remove"
                :disabled="removeDisabled(m)"
                :title="removeTitle(m)"
                :style="{ opacity: removeDisabled(m) ? 0.4 : 1, cursor: removeDisabled(m) ? 'not-allowed' : 'pointer' }"
                @click="!removeDisabled(m) && (removeTarget = m)"
              >
                <Icon name="user-minus" :size="13" />
              </button>
            </template>
          </div>
        </div>
      </div>

      <template v-if="agentMembers.length > 0">
        <div class="atl-members-section-label" style="margin-top: 18px;">Agents</div>

        <div class="atl-members-table" data-section="agents">
          <div class="atl-members-head">
            <div style="flex: 2;">Agent</div>
            <div style="flex: 0 0 120px;">Type</div>
            <div style="flex: 0 0 150px;"></div>
          </div>

          <div v-for="a in agentMembers" :key="a.id" class="atl-members-row" data-agent-row>
            <div style="flex: 2; display: flex; align-items: center; gap: 10px; min-width: 0;">
              <Avatar :agent="true" :size="26" :name="a.display" />
              <span class="atl-member-name">{{ a.display }}</span>
            </div>

            <div style="flex: 0 0 120px; display: flex; align-items: center;">
              <AgentBadge :label="agentBadgeLabel(a)" />
            </div>

            <div style="flex: 0 0 150px;"></div>
          </div>
        </div>
      </template>
    </template>

    <div class="atl-members-note">
      <Icon name="shield" :size="13" style="color: var(--c-primary);" />
      Only owners and admins can change roles or remove members. A workspace must always keep at least one owner.
    </div>

    <ConfirmDialog
      :open="removeTarget !== null"
      tone="danger"
      title="Remove this member?"
      message="They lose access to this workspace and everything in it until re-added."
      :detail="removeDetail"
      detail-icon="user"
      confirm-label="Remove member"
      confirm-icon="user-minus"
      @confirm="confirmRemove"
      @cancel="removeTarget = null"
    />

    <Teleport to="body">
      <div
        v-if="addOpen"
        class="fixed inset-0 flex items-center justify-center"
        style="background: rgba(7, 10, 15, 0.66); z-index: 300; padding: 24px;"
        @mousedown.self="closeAdd"
      >
        <div
          role="dialog"
          aria-modal="true"
          aria-label="Add member"
          data-add-dialog
          :style="{
            width: '440px',
            maxWidth: '100%',
            background: 'var(--c-raised)',
            border: '1px solid var(--c-border)',
            borderRadius: '4px',
            boxShadow: 'var(--shadow-lg)',
            overflow: 'hidden',
            fontFamily: 'var(--font-ui)',
          }"
        >
          <div class="flex items-start" style="gap: 12px; padding: 16px 16px 0;">
            <div class="flex-1 min-w-0">
              <h2 style="font-size: var(--fs-lg); font-weight: var(--fw-bold); color: var(--c-foreground); margin: 0;">
                Add member
              </h2>
              <p style="font-size: 12.5px; line-height: 1.5; color: var(--c-muted); margin: 5px 0 0;">
                Add an existing user to this workspace and choose their role.
              </p>
            </div>
            <button
              type="button"
              data-action="close-add"
              class="inline-flex items-center justify-center shrink-0 cursor-pointer"
              style="width: 22px; height: 22px; border: none; background: transparent; color: var(--c-muted); border-radius: var(--r-sm);"
              title="Close"
              aria-label="Close"
              @click="closeAdd"
            >
              <Icon name="x" :size="14" />
            </button>
          </div>

          <div style="padding: 14px 16px 0;">
            <div
              v-if="addError"
              data-add-error
              style="background: var(--c-banner-err-bg); border: 1px solid rgba(240, 113, 120, 0.5); border-radius: var(--r-md); padding: 8px 11px; margin-bottom: 12px; font-size: var(--fs-sm); color: var(--c-banner-err-fg);"
            >
              {{ addError }}
            </div>

            <div v-if="addLoading" style="font-size: 13px; color: var(--c-muted); padding: 12px 4px;">
              Loading&hellip;
            </div>

            <EmptyState
              v-else-if="assignableUsers.length === 0"
              compact
              icon="users"
              title="No one to add"
              hint="Everyone is already a member, or no other active users exist."
              data-add-empty
              style="padding: 32px 20px;"
            />

            <template v-else>
              <div class="atl-members-section-label" style="margin-bottom: 6px;">User</div>
              <div class="atl-add-user-list" role="listbox" data-add-user-list>
                <button
                  v-for="u in assignableUsers"
                  :key="u.id"
                  type="button"
                  role="option"
                  data-add-user-option
                  :aria-selected="selectedUserId === u.id"
                  class="atl-add-user-row"
                  :class="{ 'atl-add-user-row--selected': selectedUserId === u.id }"
                  @click="selectedUserId = u.id"
                >
                  <Avatar :size="24" :name="u.display_name" />
                  <span class="flex-1 min-w-0" style="display: flex; flex-direction: column; min-width: 0;">
                    <span class="atl-member-name">{{ u.display_name }}</span>
                    <span style="font-size: 11px; color: var(--c-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                      @{{ u.username }}
                    </span>
                  </span>
                  <span
                    v-if="userIsPending(u)"
                    class="atl-role-badge atl-status-pending"
                    title="Pending activation"
                  >Pending</span>
                  <Icon
                    v-if="selectedUserId === u.id"
                    name="check"
                    :size="15"
                    style="color: var(--c-primary); flex: 0 0 auto;"
                  />
                </button>
              </div>

              <div class="atl-members-section-label" style="margin: 14px 0 6px;">Role</div>
              <div data-add-role>
                <Dropdown
                  :options="addableRoles"
                  :model-value="selectedRole"
                  @change="(v) => { selectedRole = v as Role; }"
                />
              </div>
            </template>
          </div>

          <div class="flex justify-end" style="gap: 8px; padding: 18px 16px 16px;">
            <Btn variant="secondary" data-action="cancel-add" :disabled="addSubmitting" @click="closeAdd">
              Cancel
            </Btn>
            <Btn
              variant="primary"
              data-action="confirm-add"
              :disabled="addSubmitting || addLoading || selectedUserId === null"
              @click="confirmAdd"
            >
              <Icon name="user-plus" :size="14" />
              {{ addSubmitting ? 'Adding…' : 'Add member' }}
            </Btn>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.atl-panel-head {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  margin-bottom: 16px;
}

.atl-members-section-label {
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--c-muted);
  margin-bottom: 7px;
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

.atl-members-table {
  border: 1px solid var(--c-border);
  border-radius: 4px;
  overflow: hidden;
}

.atl-members-head {
  display: flex;
  align-items: center;
  height: 28px;
  padding: 0 12px;
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--c-muted);
}

.atl-members-row {
  display: flex;
  align-items: center;
  height: 46px;
  padding: 0 12px;
  border-top: 1px solid var(--c-border);
}

.atl-member-name {
  font-size: 13px;
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-you {
  flex: 0 0 auto;
  font-size: 10px;
  font-weight: var(--fw-semibold);
  color: var(--c-muted);
  border: 1px solid var(--c-border);
  background: var(--c-raised);
  border-radius: var(--r-sm);
  padding: 2px 8px;
}

.atl-role-badge {
  display: inline-block;
  font-size: 10px;
  font-weight: var(--fw-bold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  border-radius: var(--r-sm);
  padding: 2px 7px;
}

.atl-role-owner {
  color: var(--c-primary);
  border: 1px solid rgba(255, 180, 84, 0.45);
  background: rgba(255, 180, 84, 0.12);
}

.atl-role-admin {
  color: var(--c-foreground);
  border: 1px solid var(--c-border);
  background: var(--c-raised);
}

.atl-role-member {
  color: var(--c-muted);
  border: 1px solid var(--c-border);
  background: transparent;
}

.atl-status-deactivated {
  color: var(--c-danger);
  border: 1px solid color-mix(in srgb, var(--c-danger) 45%, transparent);
  background: color-mix(in srgb, var(--c-danger) 12%, transparent);
}

.atl-status-pending {
  color: var(--c-primary);
  border: 1px solid color-mix(in srgb, var(--c-primary) 45%, transparent);
  background: color-mix(in srgb, var(--c-primary) 12%, transparent);
}

.atl-member-remove {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 24px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-danger);
  cursor: pointer;
}

.atl-member-remove:hover:enabled {
  background: var(--c-raised);
}

.atl-members-note {
  display: flex;
  align-items: center;
  gap: 7px;
  margin-top: 12px;
  font-size: 12px;
  color: var(--c-muted);
}

.atl-add-user-list {
  max-height: 220px;
  overflow-y: auto;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  padding: 4px;
}

.atl-add-user-row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  height: 40px;
  padding: 0 8px;
  border: none;
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-foreground);
  cursor: pointer;
  text-align: left;
}

.atl-add-user-row:hover {
  background: var(--c-raised);
}

.atl-add-user-row--selected {
  background: var(--c-selection);
}
</style>
