<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import Avatar from '@/components/ui/Avatar.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Icon from '@/components/ui/Icon.vue';
import { useAuthStore } from '@/stores/auth';
import { useUiStore } from '@/stores/ui';
import { type PrincipalDto, useWorkspaceStore } from '@/stores/workspace';

const wsStore = useWorkspaceStore();
const auth = useAuthStore();
const ui = useUiStore();

type Role = 'owner' | 'admin' | 'member';
const ROLES: { value: Role; label: string }[] = [
  { value: 'owner', label: 'Owner' },
  { value: 'admin', label: 'Admin' },
  { value: 'member', label: 'Member' },
];

const loading = ref(false);
const busyId = ref<string | null>(null);
const removeTarget = ref<PrincipalDto | null>(null);

// This panel manages USER members only; api-key principals carry no role and
// are surfaced elsewhere (the share dialog), never here.
const userMembers = computed<PrincipalDto[]>(() =>
  wsStore.members.filter((m) => m.principal_type === 'user'),
);

const activeWs = computed(() => wsStore.activeWorkspaceSlug ?? '');

function roleOf(m: PrincipalDto): Role {
  const r = m.role;
  if (r === 'owner' || r === 'admin' || r === 'member') return r;
  return 'member';
}

function roleLabel(role: Role): string {
  return ROLES.find((r) => r.value === role)?.label ?? role;
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

function initials(m: PrincipalDto): string {
  const base = (m.display || '?').trim();
  const parts = base.split(/\s+/).filter(Boolean);
  const a = parts[0];
  const b = parts[1];
  if (a && b) return (a.charAt(0) + b.charAt(0)).toUpperCase();
  return base.slice(0, 2).toUpperCase();
}

function roleTagClass(role: Role): string {
  if (role === 'owner') return 'atl-role-owner';
  if (role === 'admin') return 'atl-role-admin';
  return 'atl-role-member';
}

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
</script>

<template>
  <div>
    <div class="atl-panel-head">
      <div class="atl-panel-title">Members</div>
      <div class="atl-panel-sub">Manage who belongs to this workspace and what they can do</div>
    </div>

    <div v-if="loading" style="font-size: 13px; color: var(--c-muted); padding: 8px;">
      Loading&hellip;
    </div>

    <div v-else-if="userMembers.length === 0" class="atl-members-empty">
      <div class="atl-members-empty-icon"><Icon name="users" :size="22" /></div>
      <div style="font-size: 14px; font-weight: var(--fw-semibold); color: var(--c-foreground);">
        No members yet
      </div>
      <div style="font-size: 12.5px; color: var(--c-muted); margin-top: 5px; max-width: 300px; line-height: 1.5;">
        Workspace members appear here with their role. Owners and admins can change roles or remove members.
      </div>
    </div>

    <div v-else class="atl-members-table">
      <div class="atl-members-head">
        <div style="flex: 2;">Member</div>
        <div style="flex: 0 0 120px;">Role</div>
        <div style="flex: 0 0 150px;"></div>
      </div>

      <div v-for="m in userMembers" :key="m.id" class="atl-members-row">
        <div style="flex: 2; display: flex; align-items: center; gap: 10px; min-width: 0;">
          <Avatar :name="initials(m)" :size="26" />
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
            <div class="atl-role-select-box">
              <select
                class="atl-role-select"
                :value="roleOf(m)"
                :disabled="roleSelectDisabled(m)"
                :title="isLastOwner(m) ? 'A workspace must keep at least one owner' : 'Change role'"
                @change="changeRole(m, ($event.target as HTMLSelectElement).value as Role)"
              >
                <option v-for="r in ROLES" :key="r.value" :value="r.value">{{ r.label }}</option>
              </select>
            </div>
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

.atl-role-select-box {
  display: flex;
  align-items: center;
  height: 24px;
  padding: 0 2px 0 8px;
  background-color: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
}

.atl-role-select {
  background: transparent;
  border: none;
  outline: none;
  color: var(--c-foreground);
  font-size: 12px;
  font-family: var(--font-ui);
  cursor: pointer;
}

.atl-role-select:disabled {
  opacity: 0.45;
  cursor: not-allowed;
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

.atl-members-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: 54px 20px;
  border: 1px dashed var(--c-border);
  border-radius: 4px;
}

.atl-members-empty-icon {
  width: 44px;
  height: 44px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--c-muted);
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  margin-bottom: 14px;
}

.atl-members-note {
  display: flex;
  align-items: center;
  gap: 7px;
  margin-top: 12px;
  font-size: 12px;
  color: var(--c-muted);
}
</style>
