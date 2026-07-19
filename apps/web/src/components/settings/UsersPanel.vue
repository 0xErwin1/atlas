<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue';
import { z } from 'zod';
import ExpandableRow from '@/components/settings/ExpandableRow.vue';
import PanelHeader from '@/components/settings/PanelHeader.vue';
import RowAction from '@/components/settings/RowAction.vue';
import SettingsTable from '@/components/settings/SettingsTable.vue';
import WorkspaceAccessEditor, { type RoleOption } from '@/components/settings/WorkspaceAccessEditor.vue';
import Avatar from '@/components/ui/Avatar.vue';
import Btn from '@/components/ui/Btn.vue';
import Chip from '@/components/ui/Chip.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import FormField from '@/components/ui/FormField.vue';
import Icon from '@/components/ui/Icon.vue';
import SecretReveal from '@/components/ui/SecretReveal.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';
import { useLoadingMap } from '@/composables/useLoadingMap';
import { formatDate, initials as nameInitials } from '@/lib/format';
import { validateForm } from '@/lib/validation';
import { workspaceRoleOptions } from '@/lib/workspaceRoles';
import { useAuthStore } from '@/stores/auth';
import { useUiStore } from '@/stores/ui';
import { activationUrl, type UserDto, useUsersStore } from '@/stores/users';
import { useWorkspaceStore } from '@/stores/workspace';

const usersStore = useUsersStore();
const auth = useAuthStore();
const ui = useUiStore();
const wsStore = useWorkspaceStore();

const currentUserIsRoot = computed(() => auth.user?.is_root === true);

type Mode = 'list' | 'new' | 'reset' | 'link';
const mode = ref<Mode>('list');

const ROLES: DropdownOption[] = workspaceRoleOptions();

const workspaceOptions = computed<DropdownOption[]>(() =>
  wsStore.adminWorkspaces.map((w) => ({ value: w.slug, label: w.name })),
);

function isPending(u: UserDto): boolean {
  return u.activated_at == null;
}

const activeRootCount = computed(
  () => usersStore.users.filter((u) => u.is_root && u.disabled_at == null).length,
);

function isSelf(u: UserDto): boolean {
  return auth.user?.id != null && u.id === auth.user.id;
}

// Mirrors server-side member-management guards: you cannot manage yourself, and
// a non-root caller cannot manage the root user.
function canManage(u: UserDto): boolean {
  if (isSelf(u)) return false;
  if (!currentUserIsRoot.value && u.is_root) return false;
  return true;
}

// Mirrors server-side disable guards.
function canDisable(u: UserDto): boolean {
  if (isSelf(u)) return false;
  if (u.is_root && u.disabled_at == null && activeRootCount.value <= 1) return false;
  if (!currentUserIsRoot.value && (u.is_root || u.is_system_admin)) return false;
  return true;
}

// Enable mirrors the disable protections that still apply when re-enabling:
// no self-management, and a non-root caller cannot manage root or system-admin
// targets. The last-active-root rule is disable-only.
function canEnable(u: UserDto): boolean {
  if (isSelf(u)) return false;
  if (!currentUserIsRoot.value && (u.is_root || u.is_system_admin)) return false;
  return true;
}

function disableTitle(u: UserDto): string {
  if (isSelf(u)) return "Can't disable yourself";
  if (u.is_root && u.disabled_at == null && activeRootCount.value <= 1)
    return "Can't disable the last active root";
  if (!currentUserIsRoot.value && (u.is_root || u.is_system_admin))
    return 'System-admins cannot disable root or peer system-admin users';
  return 'Disable user';
}

function initials(u: UserDto): string {
  return nameInitials(u.display_name || u.username);
}

onMounted(() => {
  usersStore.loadUsers();
  wsStore.loadAdminWorkspaces();
});

// ── New user ───────────────────────────────────────────────────────
const form = reactive({ username: '', display_name: '', email: '', workspace: '', role: 'member' });
const formErrors = reactive<{
  username: string | null;
  display_name: string | null;
  email: string | null;
  workspace: string | null;
  role: string | null;
}>({
  username: null,
  display_name: null,
  email: null,
  workspace: null,
  role: null,
});
const saving = ref(false);

const newUserSchema = z.object({
  username: z.string().trim().min(1, 'Username is required'),
  display_name: z.string().trim().min(1, 'Display name is required'),
  email: z.union([z.literal(''), z.string().email('Enter a valid email')]),
  workspace: z.string().min(1, 'Choose a workspace'),
  role: z.enum(['member', 'admin']),
});

function startNew(): void {
  form.username = '';
  form.display_name = '';
  form.email = '';
  form.workspace = wsStore.adminWorkspaces[0]?.slug ?? '';
  form.role = 'member';
  formErrors.username = null;
  formErrors.display_name = null;
  formErrors.email = null;
  formErrors.workspace = null;
  formErrors.role = null;
  mode.value = 'new';
}

async function submitNew(): Promise<void> {
  const result = validateForm(newUserSchema, { ...form });
  formErrors.username = result.ok ? null : (result.errors.username ?? null);
  formErrors.display_name = result.ok ? null : (result.errors.display_name ?? null);
  formErrors.email = result.ok ? null : (result.errors.email ?? null);
  formErrors.workspace = result.ok ? null : (result.errors.workspace ?? null);
  formErrors.role = result.ok ? null : (result.errors.role ?? null);
  if (!result.ok) return;

  saving.value = true;
  const created = await usersStore.createUser({
    username: result.data.username,
    display_name: result.data.display_name,
    email: result.data.email === '' ? null : result.data.email,
    workspace: result.data.workspace,
    role: result.data.role,
  });
  saving.value = false;

  if (created) {
    showLink(created.user, created.activation_link);
  } else if (usersStore.error) {
    ui.showBanner(usersStore.error, 'error');
  }
}

// ── Activation-link reveal (one-time) ──────────────────────────────
const linkTarget = ref<UserDto | null>(null);
const linkUrl = ref('');
const regenerating = ref(false);

function showLink(user: UserDto, path: string): void {
  linkTarget.value = user;
  linkUrl.value = activationUrl(path);
  mode.value = 'link';
}

async function doneLink(): Promise<void> {
  linkTarget.value = null;
  linkUrl.value = '';
  mode.value = 'list';
  await usersStore.loadUsers();
}

async function regenerateLink(u: UserDto): Promise<void> {
  regenerating.value = true;
  const path = await usersStore.regenerateActivationLink(u.id);
  regenerating.value = false;

  if (path !== null) {
    showLink(u, path);
  } else if (usersStore.error) {
    ui.showBanner(usersStore.error, 'error');
  }
}

// ── Reset password ─────────────────────────────────────────────────
const resetTarget = ref<UserDto | null>(null);
const resetForm = reactive({ password: '', confirm: '' });
const resetErrors = reactive<{ password: string | null; confirm: string | null }>({
  password: null,
  confirm: null,
});
const resetting = ref(false);

const resetSchema = z
  .object({
    password: z.string().min(8, 'Use at least 8 characters'),
    confirm: z.string().min(1, 'Confirm the new password'),
  })
  .refine((v) => v.password === v.confirm, { path: ['confirm'], message: "Passwords don't match" });

function startReset(u: UserDto): void {
  resetTarget.value = u;
  resetForm.password = '';
  resetForm.confirm = '';
  resetErrors.password = null;
  resetErrors.confirm = null;
  mode.value = 'reset';
}

async function submitReset(): Promise<void> {
  const target = resetTarget.value;
  if (target === null) return;

  const result = validateForm(resetSchema, { password: resetForm.password, confirm: resetForm.confirm });
  resetErrors.password = result.ok ? null : (result.errors.password ?? null);
  resetErrors.confirm = result.ok ? null : (result.errors.confirm ?? null);
  if (!result.ok) return;

  resetting.value = true;
  const ok = await usersStore.resetPassword(target.id, resetForm.password);
  resetting.value = false;

  if (ok) {
    ui.showBanner(`Password reset for ${target.display_name}`, 'success');
    mode.value = 'list';
  } else if (usersStore.error) {
    ui.showBanner(usersStore.error, 'error');
  }
}

// ── Manage panel (expand) ──────────────────────────────────────────
const expandedUserId = ref<string | null>(null);
const membershipsLoading = useLoadingMap();

function toggleManage(u: UserDto): void {
  if (expandedUserId.value === u.id) {
    expandedUserId.value = null;
    return;
  }

  expandedUserId.value = u.id;

  if (usersStore.memberships[u.id] === undefined) {
    void loadMemberships(u);
  }
}

async function loadMemberships(u: UserDto): Promise<void> {
  membershipsLoading.set(u.id, true);
  await usersStore.loadMemberships(u.id);
  membershipsLoading.set(u.id, false);

  if (usersStore.error) ui.showBanner(usersStore.error, 'error');
}

// ── Workspace-access editor ────────────────────────────────────────
// Root grants owner; everyone else is capped at admin (the backend rejects an
// admin granting owner with 403, so the option is hidden rather than offered).
const wsAccessOptions = computed<RoleOption[]>(() =>
  workspaceRoleOptions({ includeOwner: currentUserIsRoot.value }),
);

async function onWsAssign(u: UserDto, slug: string, role: string): Promise<void> {
  const current = usersStore.memberships[u.id]?.[slug];

  const ok =
    current === undefined || current === ''
      ? await wsStore.addMember(slug, u.id, role)
      : await wsStore.updateMemberRole(slug, u.id, role);

  if (ok) {
    await usersStore.loadMemberships(u.id);
    ui.showBanner(`${u.display_name}'s access updated`, 'success');
  } else if (wsStore.error) {
    ui.showBanner(wsStore.error, 'error');
    await usersStore.loadMemberships(u.id);
  }
}

async function onWsRemove(u: UserDto, slug: string): Promise<void> {
  const ok = await wsStore.removeMember(slug, u.id);

  if (ok) {
    await usersStore.loadMemberships(u.id);
    ui.showBanner(`${u.display_name}'s access removed`, 'success');
  } else if (wsStore.error) {
    ui.showBanner(wsStore.error, 'error');
    await usersStore.loadMemberships(u.id);
  }
}

// ── System-admin toggle (root only) ────────────────────────────────
const sysAdminTarget = ref<UserDto | null>(null);

const sysAdminPromoting = computed(() =>
  sysAdminTarget.value ? !sysAdminTarget.value.is_system_admin : false,
);

const sysAdminDetail = computed(() =>
  sysAdminTarget.value
    ? `${sysAdminTarget.value.display_name} · @${sysAdminTarget.value.username}`
    : undefined,
);

async function confirmSystemAdmin(): Promise<void> {
  const u = sysAdminTarget.value;
  sysAdminTarget.value = null;
  if (u === null) return;

  const updated = await usersStore.setSystemAdmin(u.id, !u.is_system_admin);
  if (updated) {
    ui.showBanner(
      updated.is_system_admin
        ? `${u.display_name} promoted to system-admin`
        : `${u.display_name} demoted from system-admin`,
      'success',
    );
  } else if (usersStore.error) {
    ui.showBanner(usersStore.error, 'error');
  }
}

// ── Reset password (confirm before revealing the form) ─────────────
const resetConfirmTarget = ref<UserDto | null>(null);

const resetConfirmDetail = computed(() =>
  resetConfirmTarget.value
    ? `${resetConfirmTarget.value.display_name} · @${resetConfirmTarget.value.username}`
    : undefined,
);

function onResetConfirmed(): void {
  const u = resetConfirmTarget.value;
  resetConfirmTarget.value = null;
  if (u !== null) startReset(u);
}

// ── Enable / disable ───────────────────────────────────────────────
const disableTarget = ref<UserDto | null>(null);

async function enable(u: UserDto): Promise<void> {
  const ok = await usersStore.setDisabled(u.id, false);
  if (ok) ui.showBanner(`${u.display_name} enabled`, 'success');
  else if (usersStore.error) ui.showBanner(usersStore.error, 'error');
}

async function confirmDisable(): Promise<void> {
  const target = disableTarget.value;
  disableTarget.value = null;
  if (target === null) return;

  const ok = await usersStore.setDisabled(target.id, true);
  if (ok) ui.showBanner(`${target.display_name} disabled`, 'success');
  else if (usersStore.error) ui.showBanner(usersStore.error, 'error');
}
</script>

<template>
  <div>
    <!-- New user -->
    <div v-if="mode === 'new'">
      <PanelHeader
        title="New user"
        subtitle="Creates a pending account and a one-time activation link — the user sets their own password."
      />

      <div class="flex flex-col" style="gap: 14px; max-width: 430px;">
        <FormField
          label="Username"
          :model-value="form.username"
          mono
          :error="formErrors.username"
          @update:model-value="(v) => { form.username = v; formErrors.username = null; }"
        />
        <FormField
          label="Display name"
          :model-value="form.display_name"
          :error="formErrors.display_name"
          @update:model-value="(v) => { form.display_name = v; formErrors.display_name = null; }"
        />
        <FormField
          label="Email"
          type="email"
          :model-value="form.email"
          mono
          helper="Optional — used for password recovery only."
          :error="formErrors.email"
          @update:model-value="(v) => { form.email = v; formErrors.email = null; }"
        />

        <FormField label="Workspace" :error="formErrors.workspace">
          <template #control>
            <Dropdown
              data-new-workspace
              :options="workspaceOptions"
              :model-value="form.workspace"
              placeholder="No workspaces available"
              @change="(v) => { form.workspace = v; formErrors.workspace = null; }"
            />
          </template>
        </FormField>

        <FormField label="Role">
          <template #control>
            <Dropdown
              data-new-role
              :options="ROLES"
              :model-value="form.role"
              @change="(v) => { form.role = v; }"
            />
          </template>
        </FormField>
      </div>

      <div class="flex" style="gap: 8px; margin-top: 20px;">
        <Btn variant="primary" :disabled="saving" @click="submitNew">
          <Icon name="plus" :size="14" />Create user
        </Btn>
        <Btn variant="secondary" @click="mode = 'list'">Cancel</Btn>
      </div>
    </div>

    <!-- Activation link shown exactly once -->
    <div v-else-if="mode === 'link' && linkTarget">
      <PanelHeader
        title="Activation link"
        subtitle="Share this with the user so they can set their password and sign in"
      />

      <div class="atl-reset-id">
        <Avatar :name="initials(linkTarget)" :size="30" />
        <div style="min-width: 0;">
          <div style="font-size: 13px; font-weight: var(--fw-semibold); color: var(--c-foreground);">
            {{ linkTarget.display_name }}
          </div>
          <div style="font-size: 11.5px; color: var(--c-muted); font-family: var(--font-mono);">
            @{{ linkTarget.username }}
          </div>
        </div>
      </div>

      <SecretReveal
        :value="linkUrl"
        warning="Copy this now — it won't be shown again. Regenerating creates a new link and voids this one."
      />

      <div class="flex" style="justify-content: flex-end; margin-top: 16px;">
        <Btn variant="secondary" @click="doneLink">Done</Btn>
      </div>
    </div>

    <!-- Reset password -->
    <div v-else-if="mode === 'reset' && resetTarget">
      <PanelHeader title="Reset password" subtitle="Set a new password for another user" />

      <div class="atl-reset-id">
        <Avatar :name="initials(resetTarget)" :size="30" />
        <div style="min-width: 0;">
          <div style="font-size: 13px; font-weight: var(--fw-semibold); color: var(--c-foreground);">
            {{ resetTarget.display_name }}
          </div>
          <div style="font-size: 11.5px; color: var(--c-muted); font-family: var(--font-mono);">
            @{{ resetTarget.username }}
          </div>
        </div>
      </div>

      <div class="flex flex-col" style="gap: 14px; max-width: 430px;">
        <FormField
          label="New password"
          type="password"
          :model-value="resetForm.password"
          :error="resetErrors.password"
          @update:model-value="(v) => { resetForm.password = v; resetErrors.password = null; }"
        />
        <FormField
          label="Confirm new password"
          type="password"
          :model-value="resetForm.confirm"
          helper="The user isn't notified — share the new password with them directly."
          :error="resetErrors.confirm"
          @update:model-value="(v) => { resetForm.confirm = v; resetErrors.confirm = null; }"
        />
      </div>

      <div class="flex" style="gap: 8px; margin-top: 20px;">
        <Btn variant="primary" :disabled="resetting" @click="submitReset">
          <Icon name="key" :size="14" />Set password
        </Btn>
        <Btn variant="secondary" @click="mode = 'list'">Cancel</Btn>
      </div>
    </div>

    <!-- List -->
    <div v-else>
      <PanelHeader title="Users" subtitle="Create and manage Atlas accounts">
        <template #actions>
          <Btn variant="primary" @click="startNew"><Icon name="plus" :size="14" />New user</Btn>
        </template>
      </PanelHeader>

      <div v-if="usersStore.loading" style="font-size: 13px; color: var(--c-muted); padding: 8px;">
        Loading…
      </div>

      <SettingsTable v-else>
        <template #head>
          <div style="flex: 2;">User</div>
          <div style="flex: 0 0 130px;">Status</div>
          <div style="flex: 1;">Created</div>
          <div style="flex: 0 0 220px;"></div>
        </template>

        <ExpandableRow
          v-for="u in usersStore.users"
          :key="u.id"
          :expanded="expandedUserId === u.id"
          :expandable="canManage(u)"
          :style="{ height: '46px', opacity: u.disabled_at ? 0.72 : 1, '--erow-actions-basis': '220px' }"
          data-user-row
          @toggle="toggleManage(u)"
        >
          <template #summary>
            <div style="flex: 2; display: flex; align-items: center; gap: 10px; min-width: 0;">
              <Avatar :name="initials(u)" :size="26" />
              <div style="min-width: 0;">
                <div class="flex items-center" style="gap: 6px;">
                  <span style="font-size: 13px; font-weight: var(--fw-semibold); color: var(--c-foreground);">
                    {{ u.display_name }}
                  </span>
                  <span v-if="u.is_root" class="atl-tag-root">ROOT</span>
                  <span v-else-if="u.is_system_admin" class="atl-tag-sysadmin">SYSADMIN</span>
                </div>
                <div style="font-size: 11.5px; color: var(--c-muted); font-family: var(--font-mono);">
                  @{{ u.username }}
                </div>
              </div>
            </div>
            <div style="flex: 0 0 130px;">
              <Chip v-if="u.disabled_at" tone="neutral">Disabled</Chip>
              <Chip v-else-if="isPending(u)" tone="warning">Pending</Chip>
              <Chip v-else tone="success">Active</Chip>
            </div>
            <div style="flex: 1; font-size: 12px; color: var(--c-muted);">{{ formatDate(u.created_at) }}</div>
          </template>

          <template #actions>
            <span v-if="isSelf(u)" class="atl-you">you</span>
            <template v-else>
              <template v-if="u.disabled_at">
                <RowAction v-if="canEnable(u)" data-action="enable" @click="enable(u)">
                  Enable
                </RowAction>
              </template>
              <RowAction
                v-else
                tone="danger"
                data-action="disable"
                :disabled="!canDisable(u)"
                :title="disableTitle(u)"
                @click="disableTarget = u"
              >
                Disable
              </RowAction>
            </template>
          </template>

          <template #panel>
            <div class="atl-manage-stack" data-manage-panel>
              <div class="atl-user-identity">
                <Avatar :name="initials(u)" :size="30" />
                <div style="min-width: 0;">
                  <div class="atl-identity-name">{{ u.display_name }}</div>
                  <div class="atl-identity-handle">@{{ u.username }}</div>
                </div>
              </div>

              <!-- System admin -->
              <div v-if="currentUserIsRoot && !u.is_root" class="atl-manage-block">
                <ToggleSwitch
                  :model-value="u.is_system_admin"
                  label="System admin"
                  aria-label="System admin"
                  copy="Full platform access — manage users, every workspace, and audit logs."
                  data-action="toggle-sysadmin"
                  @update:model-value="sysAdminTarget = u"
                />
              </div>

              <!-- Account actions -->
              <div class="atl-manage-block">
                <div class="atl-manage-label">Account</div>
                <div class="atl-manage-actions">
                  <RowAction
                    v-if="isPending(u) && !u.disabled_at"
                    data-action="regenerate-link"
                    :disabled="regenerating"
                    @click="regenerateLink(u)"
                  >
                    <Icon name="link" :size="13" />
                    Regenerate activation link
                  </RowAction>
                  <RowAction
                    v-else-if="canManage(u)"
                    data-action="reset-password"
                    @click="resetConfirmTarget = u"
                  >
                    <Icon name="key" :size="13" />
                    Reset password
                  </RowAction>
                </div>
              </div>

              <!-- Workspace access -->
              <div class="atl-manage-block">
                <div class="atl-manage-label">Workspace access</div>
                <div v-if="membershipsLoading.isLoading(u.id)" class="atl-grants-loading">
                  Loading access&hellip;
                </div>
                <WorkspaceAccessEditor
                  v-else-if="canManage(u)"
                  :workspaces="wsStore.adminWorkspaces"
                  :roles="usersStore.memberships[u.id] ?? {}"
                  :options="wsAccessOptions"
                  @assign="(slug, role) => onWsAssign(u, slug, role)"
                  @remove="(slug) => onWsRemove(u, slug)"
                />
              </div>
            </div>
          </template>
        </ExpandableRow>
      </SettingsTable>

      <div class="atl-users-note">
        <Icon name="shield" :size="13" style="color: var(--c-primary);" />
        You can't disable yourself or the last remaining root. Open Manage to promote or demote system-admin access (root only) and edit a user's workspace access.
      </div>
    </div>

    <ConfirmDialog
      :open="disableTarget !== null"
      tone="warning"
      :width="460"
      title="Disable this user?"
      message="They're signed out everywhere and can no longer access Atlas until re-enabled."
      :detail="disableTarget ? `${disableTarget.display_name} · @${disableTarget.username}` : undefined"
      detail-icon="user"
      note="Disabling them also disables the API keys they created — agents using those keys lose access immediately. Re-enabling restores both."
      confirm-label="Disable user"
      confirm-icon="lock"
      @confirm="confirmDisable"
      @cancel="disableTarget = null"
    />

    <ConfirmDialog
      :open="sysAdminTarget !== null"
      tone="primary"
      :width="460"
      :title="sysAdminPromoting ? 'Promote to system-admin?' : 'Remove system-admin?'"
      :message="
        sysAdminPromoting
          ? 'Grants full platform access — manage every user, every workspace, and the audit logs.'
          : 'Revokes platform-wide access. They keep only their own workspace memberships.'
      "
      :detail="sysAdminDetail"
      detail-icon="shield"
      :confirm-label="sysAdminPromoting ? 'Promote' : 'Remove access'"
      :confirm-icon="sysAdminPromoting ? 'shield' : 'shield-off'"
      @confirm="confirmSystemAdmin"
      @cancel="sysAdminTarget = null"
    />

    <ConfirmDialog
      :open="resetConfirmTarget !== null"
      tone="warning"
      :width="460"
      title="Reset this user's password?"
      :message="
        resetConfirmTarget
          ? `Sets a new password for ${resetConfirmTarget.display_name} directly. They are NOT notified — you'll share the new password with them.`
          : undefined
      "
      :detail="resetConfirmDetail"
      detail-icon="user"
      confirm-label="Continue"
      confirm-icon="key"
      @confirm="onResetConfirmed"
      @cancel="resetConfirmTarget = null"
    />
  </div>
</template>

<style scoped>
.atl-tag-root {
  font-size: 9.5px;
  font-weight: var(--fw-bold);
  letter-spacing: 0.06em;
  color: var(--c-primary);
  border: 1px solid rgba(255, 180, 84, 0.45);
  background: rgba(255, 180, 84, 0.12);
  border-radius: var(--r-sm);
  padding: 1px 5px;
  font-family: var(--font-mono);
}

.atl-tag-sysadmin {
  font-size: 9.5px;
  font-weight: var(--fw-bold);
  letter-spacing: 0.06em;
  color: var(--c-muted);
  border: 1px solid var(--c-border);
  background: var(--c-raised);
  border-radius: var(--r-sm);
  padding: 1px 5px;
  font-family: var(--font-mono);
}

.atl-manage-stack {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.atl-user-identity {
  display: flex;
  align-items: center;
  gap: 10px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--c-border);
}

.atl-identity-name {
  font-size: 13px;
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-identity-handle {
  font-size: 11.5px;
  color: var(--c-muted);
  font-family: var(--font-mono);
}

.atl-manage-block {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.atl-manage-label {
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--c-muted);
}

.atl-manage-actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.atl-grants-loading {
  font-size: 12px;
  color: var(--c-muted);
  padding: 4px 0;
}

.atl-you {
  font-size: 10px;
  font-weight: var(--fw-semibold);
  color: var(--c-muted);
  border: 1px solid var(--c-border);
  background: var(--c-raised);
  border-radius: var(--r-sm);
  padding: 2px 8px;
}

.atl-users-note {
  display: flex;
  align-items: center;
  gap: 7px;
  margin-top: 12px;
  font-size: 12px;
  color: var(--c-muted);
}

.atl-reset-id {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: 4px;
  margin-bottom: 18px;
}
</style>
