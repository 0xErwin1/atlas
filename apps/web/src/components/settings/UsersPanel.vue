<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue';
import { z } from 'zod';
import ExpandableRow from '@/components/settings/ExpandableRow.vue';
import SettingsTable from '@/components/settings/SettingsTable.vue';
import WorkspaceAccessEditor, { type RoleOption } from '@/components/settings/WorkspaceAccessEditor.vue';
import Avatar from '@/components/ui/Avatar.vue';
import Btn from '@/components/ui/Btn.vue';
import Chip from '@/components/ui/Chip.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import FormField from '@/components/ui/FormField.vue';
import Icon from '@/components/ui/Icon.vue';
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

// Mirrors server-side disable guards.
function canDisable(u: UserDto): boolean {
  if (isSelf(u)) return false;
  if (u.is_root && u.disabled_at == null && activeRootCount.value <= 1) return false;
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
const linkCopied = ref(false);
const regenerating = ref(false);

function showLink(user: UserDto, path: string): void {
  linkTarget.value = user;
  linkUrl.value = activationUrl(path);
  linkCopied.value = false;
  mode.value = 'link';
}

async function copyLink(): Promise<void> {
  if (linkUrl.value === '') return;
  try {
    await navigator.clipboard.writeText(linkUrl.value);
    linkCopied.value = true;
  } catch {
    ui.showBanner('Clipboard is not available', 'error');
  }
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
const membershipsLoading = ref<Record<string, boolean>>({});

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
  membershipsLoading.value = { ...membershipsLoading.value, [u.id]: true };
  await usersStore.loadMemberships(u.id);
  membershipsLoading.value = { ...membershipsLoading.value, [u.id]: false };

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
      <div class="atl-panel-head">
        <div class="atl-panel-title">New user</div>
        <div class="atl-panel-sub">
          Creates a pending account and a one-time activation link — the user sets their own password.
        </div>
      </div>

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

        <div class="atl-field">
          <label class="atl-field-label">Workspace</label>
          <Dropdown
            data-new-workspace
            :options="workspaceOptions"
            :model-value="form.workspace"
            placeholder="No workspaces available"
            @change="(v) => { form.workspace = v; formErrors.workspace = null; }"
          />
          <div v-if="formErrors.workspace" class="atl-select-error">
            <Icon name="triangle-alert" :size="12" />{{ formErrors.workspace }}
          </div>
        </div>

        <div class="atl-field">
          <label class="atl-field-label">Role</label>
          <Dropdown
            data-new-role
            :options="ROLES"
            :model-value="form.role"
            @change="(v) => { form.role = v; }"
          />
        </div>
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
      <div class="atl-panel-head">
        <div class="atl-panel-title">Activation link</div>
        <div class="atl-panel-sub">Share this with the user so they can set their password and sign in</div>
      </div>

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

      <div class="atl-secret-box">
        <div class="atl-secret-warn">
          <Icon name="triangle-alert" :size="14" style="flex: 0 0 auto;" />
          Copy this now — it won't be shown again. Regenerating creates a new link and voids this one.
        </div>
        <div style="padding: 14px; background: var(--c-raised);">
          <div class="flex items-center" style="gap: 8px;">
            <div class="atl-secret-value">{{ linkUrl }}</div>
            <button type="button" class="atl-copybtn" @click="copyLink">
              <Icon :name="linkCopied ? 'check' : 'copy'" :size="14" />{{ linkCopied ? 'Copied' : 'Copy' }}
            </button>
          </div>
        </div>
      </div>

      <div class="flex" style="justify-content: flex-end; margin-top: 16px;">
        <Btn variant="secondary" @click="doneLink">Done</Btn>
      </div>
    </div>

    <!-- Reset password -->
    <div v-else-if="mode === 'reset' && resetTarget">
      <div class="atl-panel-head">
        <div class="atl-panel-title">Reset password</div>
        <div class="atl-panel-sub">Set a new password for another user</div>
      </div>

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
      <div class="atl-panel-head atl-panel-head-row">
        <div>
          <div class="atl-panel-title">Users</div>
          <div class="atl-panel-sub">Create and manage Atlas accounts</div>
        </div>
        <Btn variant="primary" @click="startNew"><Icon name="plus" :size="14" />New user</Btn>
      </div>

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
          :expandable="!isSelf(u)"
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
              <button v-if="u.disabled_at" type="button" class="atl-rowact" @click="enable(u)">
                Enable
              </button>
              <button
                v-else
                type="button"
                class="atl-revoke"
                :disabled="!canDisable(u)"
                :title="disableTitle(u)"
                :style="{ opacity: canDisable(u) ? 1 : 0.4, cursor: canDisable(u) ? 'pointer' : 'not-allowed' }"
                @click="canDisable(u) && (disableTarget = u)"
              >
                Disable
              </button>
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
                <div class="atl-global-toggle">
                  <button
                    type="button"
                    role="switch"
                    class="atl-switch"
                    :class="{ 'atl-switch--on': u.is_system_admin }"
                    :aria-checked="u.is_system_admin"
                    aria-label="System admin"
                    data-action="toggle-sysadmin"
                    @click="sysAdminTarget = u"
                  >
                    <span class="atl-switch-knob" />
                  </button>
                  <div class="atl-global-copy">
                    <div class="atl-global-label">System admin</div>
                    <div class="atl-global-help">
                      Full platform access — manage users, every workspace, and audit logs.
                    </div>
                  </div>
                </div>
              </div>

              <!-- Account actions -->
              <div class="atl-manage-block">
                <div class="atl-manage-label">Account</div>
                <div class="atl-manage-actions">
                  <button
                    v-if="isPending(u) && !u.disabled_at"
                    type="button"
                    class="atl-manage-btn"
                    data-action="regenerate-link"
                    :disabled="regenerating"
                    @click="regenerateLink(u)"
                  >
                    <Icon name="link" :size="13" />
                    Regenerate activation link
                  </button>
                  <button
                    v-else
                    type="button"
                    class="atl-manage-btn"
                    data-action="reset-password"
                    @click="resetConfirmTarget = u"
                  >
                    <Icon name="key" :size="13" />
                    Reset password
                  </button>
                </div>
              </div>

              <!-- Workspace access -->
              <div class="atl-manage-block">
                <div class="atl-manage-label">Workspace access</div>
                <div v-if="membershipsLoading[u.id]" class="atl-grants-loading">
                  Loading access&hellip;
                </div>
                <WorkspaceAccessEditor
                  v-else
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
.atl-panel-head {
  margin-bottom: 16px;
}

.atl-panel-head-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
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

.atl-manage-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 11px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-foreground);
  font-size: 12px;
  cursor: pointer;
}

.atl-manage-btn:hover:enabled {
  background: var(--c-raised);
}

.atl-manage-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.atl-grants-loading {
  font-size: 12px;
  color: var(--c-muted);
  padding: 4px 0;
}

/* Toggle switch — identical markup/styling to ApiKeysPanel "Global agent". */
.atl-global-toggle {
  display: flex;
  align-items: flex-start;
  gap: 10px;
}

.atl-switch {
  flex: 0 0 auto;
  position: relative;
  width: 34px;
  height: 20px;
  margin-top: 1px;
  padding: 0;
  border: 1px solid var(--c-border);
  border-radius: 9999px;
  background: var(--c-input);
  cursor: pointer;
  transition: background 0.15s, border-color 0.15s;
}

.atl-switch--on {
  background: var(--c-agent);
  border-color: var(--c-agent);
}

.atl-switch:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.atl-switch-knob {
  position: absolute;
  top: 50%;
  left: 2px;
  width: 14px;
  height: 14px;
  border-radius: 9999px;
  background: var(--c-foreground);
  transform: translateY(-50%);
  transition: left 0.15s;
}

.atl-switch--on .atl-switch-knob {
  left: 17px;
  background: var(--c-on-agent, #fff);
}

.atl-global-copy {
  min-width: 0;
}

.atl-global-label {
  font-size: 12.5px;
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-global-help {
  font-size: 11.5px;
  color: var(--c-muted);
  line-height: 1.45;
  margin-top: 2px;
  max-width: 440px;
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

.atl-rowact {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 24px;
  padding: 0 8px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-foreground);
  cursor: pointer;
  font-size: 12px;
}

.atl-rowact:hover {
  background: var(--c-raised);
}

.atl-revoke {
  height: 24px;
  padding: 0 10px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-danger);
  cursor: pointer;
  font-size: 12px;
}

.atl-revoke:hover:enabled {
  background: var(--c-raised);
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

.atl-rowact:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.atl-field {
  display: flex;
  flex-direction: column;
}

.atl-field-label {
  display: block;
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--c-muted);
  margin-bottom: 5px;
}

.atl-select-error {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 11.5px;
  color: var(--c-danger);
  margin-top: 5px;
}

.atl-secret-box {
  border: 1px solid rgba(255, 180, 84, 0.45);
  border-radius: 4px;
  overflow: hidden;
}

.atl-secret-warn {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 9px 12px;
  background: rgba(255, 180, 84, 0.12);
  border-bottom: 1px solid rgba(255, 180, 84, 0.45);
  color: var(--c-primary);
  font-size: 12.5px;
  font-weight: var(--fw-semibold);
}

.atl-secret-value {
  flex: 1;
  min-width: 0;
  height: 36px;
  display: flex;
  align-items: center;
  padding: 0 11px;
  background: var(--c-background);
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg);
  font-family: var(--font-mono);
  font-size: 13px;
  color: var(--c-foreground);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-copybtn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 36px;
  padding: 0 12px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: var(--c-raised);
  color: var(--c-foreground);
  cursor: pointer;
  font-size: 12.5px;
}
</style>
