<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue';
import { z } from 'zod';
import Avatar from '@/components/ui/Avatar.vue';
import Btn from '@/components/ui/Btn.vue';
import Chip from '@/components/ui/Chip.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import FormField from '@/components/ui/FormField.vue';
import Icon from '@/components/ui/Icon.vue';
import { validateForm } from '@/lib/validation';
import { useAuthStore } from '@/stores/auth';
import { useUiStore } from '@/stores/ui';
import { type UserDto, useUsersStore } from '@/stores/users';

const usersStore = useUsersStore();
const auth = useAuthStore();
const ui = useUiStore();

type Mode = 'list' | 'new' | 'reset';
const mode = ref<Mode>('list');

const activeRootCount = computed(
  () => usersStore.users.filter((u) => u.is_root && u.disabled_at == null).length,
);

function isSelf(u: UserDto): boolean {
  return auth.user?.id != null && u.id === auth.user.id;
}

// A root can't be disabled if it is the last active one — mirrors the server guard.
function canDisable(u: UserDto): boolean {
  if (isSelf(u)) return false;
  if (u.is_root && u.disabled_at == null && activeRootCount.value <= 1) return false;
  return true;
}

function initials(u: UserDto): string {
  const base = (u.display_name || u.username || '?').trim();
  const parts = base.split(/\s+/).filter(Boolean);
  const a = parts[0];
  const b = parts[1];
  if (a && b) return (a.charAt(0) + b.charAt(0)).toUpperCase();
  return base.slice(0, 2).toUpperCase();
}

function fmtDate(iso: string | null | undefined): string {
  if (!iso) return '—';
  return new Date(iso).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: '2-digit' });
}

onMounted(() => usersStore.loadUsers());

// ── New user ───────────────────────────────────────────────────────
const form = reactive({ username: '', display_name: '', email: '', password: '' });
const formErrors = reactive<Record<keyof typeof form, string | null>>({
  username: null,
  display_name: null,
  email: null,
  password: null,
});
const saving = ref(false);

const newUserSchema = z.object({
  username: z.string().trim().min(1, 'Username is required'),
  display_name: z.string().trim().min(1, 'Display name is required'),
  email: z.union([z.literal(''), z.string().email('Enter a valid email')]),
  password: z.string().min(8, 'Use at least 8 characters'),
});

function startNew(): void {
  form.username = '';
  form.display_name = '';
  form.email = '';
  form.password = '';
  formErrors.username = null;
  formErrors.display_name = null;
  formErrors.email = null;
  formErrors.password = null;
  mode.value = 'new';
}

async function submitNew(): Promise<void> {
  const result = validateForm(newUserSchema, { ...form });
  formErrors.username = result.ok ? null : (result.errors.username ?? null);
  formErrors.display_name = result.ok ? null : (result.errors.display_name ?? null);
  formErrors.email = result.ok ? null : (result.errors.email ?? null);
  formErrors.password = result.ok ? null : (result.errors.password ?? null);
  if (!result.ok) return;

  saving.value = true;
  const created = await usersStore.createUser({
    username: result.data.username,
    display_name: result.data.display_name,
    email: result.data.email === '' ? null : result.data.email,
    password: result.data.password,
  });
  saving.value = false;

  if (created) {
    ui.showBanner('User created', 'success');
    mode.value = 'list';
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
  .refine((v) => v.password === v.confirm, { path: ['confirm'], message: 'Passwords don’t match' });

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
        <div class="atl-panel-sub">Atlas accounts use a username — email is optional</div>
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
        <FormField
          label="Initial password"
          type="password"
          :model-value="form.password"
          helper="The user can change this from their Account tab after first sign-in."
          :error="formErrors.password"
          @update:model-value="(v) => { form.password = v; formErrors.password = null; }"
        />
      </div>

      <div class="flex" style="gap: 8px; margin-top: 20px;">
        <Btn variant="primary" :disabled="saving" @click="submitNew">
          <Icon name="plus" :size="14" />Create user
        </Btn>
        <Btn variant="secondary" @click="mode = 'list'">Cancel</Btn>
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
          helper="The user isn’t notified — share the new password with them directly."
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

      <div v-else class="atl-users-table">
        <div class="atl-users-head">
          <div style="flex: 2;">User</div>
          <div style="flex: 0 0 130px;">Status</div>
          <div style="flex: 1;">Created</div>
          <div style="flex: 0 0 156px;"></div>
        </div>
        <div
          v-for="u in usersStore.users"
          :key="u.id"
          class="atl-users-row"
          :style="{ opacity: u.disabled_at ? 0.72 : 1 }"
        >
          <div style="flex: 2; display: flex; align-items: center; gap: 10px; min-width: 0;">
            <Avatar :name="initials(u)" :size="26" />
            <div style="min-width: 0;">
              <div class="flex items-center" style="gap: 6px;">
                <span style="font-size: 13px; font-weight: var(--fw-semibold); color: var(--c-foreground);">
                  {{ u.display_name }}
                </span>
                <span v-if="u.is_root" class="atl-tag-root">ROOT</span>
              </div>
              <div style="font-size: 11.5px; color: var(--c-muted); font-family: var(--font-mono);">
                @{{ u.username }}
              </div>
            </div>
          </div>
          <div style="flex: 0 0 130px;">
            <Chip v-if="u.disabled_at" tone="neutral">Disabled</Chip>
            <Chip v-else tone="success">Active</Chip>
          </div>
          <div style="flex: 1; font-size: 12px; color: var(--c-muted);">{{ fmtDate(u.created_at) }}</div>
          <div style="flex: 0 0 156px; display: flex; justify-content: flex-end; gap: 6px;">
            <span v-if="isSelf(u)" class="atl-you">you</span>
            <template v-else>
              <button type="button" class="atl-rowact" title="Reset password" @click="startReset(u)">
                <Icon name="key" :size="13" />
              </button>
              <button v-if="u.disabled_at" type="button" class="atl-rowact" @click="enable(u)">
                Enable
              </button>
              <button
                v-else
                type="button"
                class="atl-revoke"
                :disabled="!canDisable(u)"
                :title="canDisable(u) ? 'Disable user' : 'Can’t disable the last active root'"
                :style="{ opacity: canDisable(u) ? 1 : 0.4, cursor: canDisable(u) ? 'pointer' : 'not-allowed' }"
                @click="canDisable(u) && (disableTarget = u)"
              >
                Disable
              </button>
            </template>
          </div>
        </div>
      </div>

      <div class="atl-users-note">
        <Icon name="shield" :size="13" style="color: var(--c-primary);" />
        You can’t disable yourself or the last remaining root. Use the key icon to reset another user’s password.
      </div>
    </div>

    <ConfirmDialog
      :open="disableTarget !== null"
      tone="warning"
      title="Disable this user?"
      message="They are signed out everywhere and can no longer access Atlas until re-enabled."
      :detail="disableTarget ? `${disableTarget.display_name} · @${disableTarget.username}` : undefined"
      detail-icon="user"
      note="Any API keys they created stop working while they are disabled; re-enabling restores access."
      confirm-label="Disable user"
      confirm-icon="lock"
      @confirm="confirmDisable"
      @cancel="disableTarget = null"
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

.atl-users-table {
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg);
  overflow: hidden;
}

.atl-users-head {
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

.atl-users-row {
  display: flex;
  align-items: center;
  height: 46px;
  padding: 0 12px;
  border-top: 1px solid var(--c-border);
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
  border-radius: var(--r-lg);
  margin-bottom: 18px;
}
</style>
