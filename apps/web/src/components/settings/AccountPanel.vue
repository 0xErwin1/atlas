<script setup lang="ts">
import { computed, reactive, ref } from 'vue';
import { useRouter } from 'vue-router';
import { z } from 'zod';
import Avatar from '@/components/ui/Avatar.vue';
import Btn from '@/components/ui/Btn.vue';
import FormField from '@/components/ui/FormField.vue';
import Icon from '@/components/ui/Icon.vue';
import SegmentedControl, { type SegmentedOption } from '@/components/ui/SegmentedControl.vue';
import { useProblem } from '@/composables/useProblem';
import { initials as nameInitials } from '@/lib/format';
import { validateForm } from '@/lib/validation';
import { getPlatformTransport } from '@/platform/transport';
import { type Problem, useAuthStore } from '@/stores/auth';
import { type Theme, useUiStore } from '@/stores/ui';

const auth = useAuthStore();
const ui = useUiStore();
const router = useRouter();
const transport = getPlatformTransport();

const displayName = computed(() => auth.user?.display_name ?? auth.user?.username ?? 'Account');
const username = computed(() => auth.user?.username ?? '');
const isRoot = computed(() => auth.user?.is_root === true);

const initials = computed(() => nameInitials(auth.user?.display_name ?? auth.user?.username));
const serverOrigin = ref('https://atlas.iperez.dev');
const serverOriginError = ref<string | null>(null);
const serverOriginSaving = ref(false);

if (transport.isDesktop) {
  void transport.getOrigin().then(({ data }) => {
    if (data !== undefined) serverOrigin.value = data.origin;
  });
}

async function updateServerOrigin(): Promise<void> {
  serverOriginError.value = null;
  serverOriginSaving.value = true;
  const result = await transport.setOrigin(serverOrigin.value);
  serverOriginSaving.value = false;

  if (result.error || result.data === undefined) {
    serverOriginError.value =
      typeof result.error === 'string' ? result.error : 'Unable to save the Atlas server URL';
    return;
  }

  serverOrigin.value = result.data.origin;
  await auth.clearUser();
  await router.push({ name: 'login' });
}

function problemText(problem: Problem): string {
  const p = useProblem(problem);
  return p.hint ?? p.message;
}

// ── Email ──────────────────────────────────────────────────────────
const email = ref(auth.user?.email ?? '');
const emailError = ref<string | null>(null);
const emailSaving = ref(false);

const emailSchema = z.object({ email: z.string().trim().email('Enter a valid email') });

async function updateEmail(): Promise<void> {
  emailError.value = null;

  const result = validateForm(emailSchema, { email: email.value });
  if (!result.ok) {
    emailError.value = result.errors.email ?? null;
    return;
  }

  emailSaving.value = true;
  const res = await auth.updateProfile({ email: result.data.email });
  emailSaving.value = false;

  if (res.ok) ui.showBanner('Email updated', 'success');
  else if (res.problem) emailError.value = problemText(res.problem);
}

// ── Change password ────────────────────────────────────────────────
const pw = reactive({ current: '', next: '', confirm: '' });
const pwErrors = reactive<{ current: string | null; next: string | null; confirm: string | null }>({
  current: null,
  next: null,
  confirm: null,
});
const pwSaving = ref(false);

const passwordSchema = z
  .object({
    current: z.string().min(1, 'Enter your current password'),
    next: z.string().min(8, 'Use at least 8 characters'),
    confirm: z.string().min(1, 'Confirm your new password'),
  })
  .refine((v) => v.next === v.confirm, { path: ['confirm'], message: 'Passwords don’t match' });

async function updatePassword(): Promise<void> {
  pwErrors.current = null;
  pwErrors.next = null;
  pwErrors.confirm = null;

  const result = validateForm(passwordSchema, { current: pw.current, next: pw.next, confirm: pw.confirm });
  if (!result.ok) {
    pwErrors.current = result.errors.current ?? null;
    pwErrors.next = result.errors.next ?? null;
    pwErrors.confirm = result.errors.confirm ?? null;
    return;
  }

  pwSaving.value = true;
  const res = await auth.changePassword({ current_password: pw.current, new_password: pw.next });
  pwSaving.value = false;

  if (res.ok) {
    pw.current = '';
    pw.next = '';
    pw.confirm = '';
    ui.showBanner('Password updated', 'success');
  } else if (res.problem) {
    // The API returns 401 when the current password is wrong.
    pwErrors.current =
      res.problem.status === 401 ? 'Current password is incorrect' : problemText(res.problem);
  }
}

// ── Appearance ─────────────────────────────────────────────────────
const THEME_OPTIONS: (SegmentedOption & { value: Theme })[] = [
  { value: 'dark', label: 'Dark', icon: 'moon' },
  { value: 'light', label: 'Light', icon: 'sun' },
];

function selectTheme(value: string): void {
  const option = THEME_OPTIONS.find((candidate) => candidate.value === value);
  if (option !== undefined) ui.setTheme(option.value);
}

// ── Sign out ───────────────────────────────────────────────────────
async function signOut(): Promise<void> {
  await auth.logout();
  router.push({ name: 'login' });
}
</script>

<template>
  <div>
    <div class="atl-id-card">
      <Avatar :name="initials" :size="44" :agent="false" />
      <div style="flex: 1; min-width: 0;">
        <div class="flex items-center" style="gap: 8px;">
          <span style="font-size: 15px; font-weight: var(--fw-bold); color: var(--c-foreground);">
            {{ displayName }}
          </span>
          <span class="atl-tag">USER</span>
          <span v-if="isRoot" class="atl-tag atl-tag-root">ROOT</span>
        </div>
        <div style="font-size: 12.5px; color: var(--c-muted); font-family: var(--font-mono); margin-top: 3px;">
          @{{ username }}
        </div>
      </div>
    </div>

    <template v-if="transport.isDesktop">
      <div class="atl-sec-title">Atlas server</div>
      <div class="atl-inline-field">
        <div style="flex: 1; min-width: 0;">
          <FormField
            type="text"
            :model-value="serverOrigin"
            placeholder="https://atlas.iperez.dev"
            mono
            :error="serverOriginError"
            @update:model-value="(value) => { serverOrigin = value; serverOriginError = null; }"
          />
        </div>
        <Btn variant="secondary" :disabled="serverOriginSaving" @click="updateServerOrigin">
          Change
        </Btn>
      </div>
    </template>

    <div class="atl-sec-title">Email</div>
    <div class="atl-inline-field">
      <div style="flex: 1; min-width: 0;">
        <FormField
          :model-value="email"
          type="email"
          placeholder="you@example.com"
          mono
          :error="emailError"
          @update:model-value="(v) => { email = v; emailError = null; }"
        />
      </div>
      <Btn variant="secondary" :disabled="emailSaving" @click="updateEmail">
        Update
      </Btn>
    </div>

    <div class="atl-sec-title">Change password</div>
    <div class="flex flex-col" style="gap: 12px; max-width: 430px; margin-bottom: 14px;">
      <FormField
        label="Current password"
        type="password"
        :model-value="pw.current"
        autocomplete="current-password"
        :error="pwErrors.current"
        @update:model-value="(v) => { pw.current = v; pwErrors.current = null; }"
      />
      <FormField
        label="New password"
        type="password"
        :model-value="pw.next"
        autocomplete="new-password"
        :error="pwErrors.next"
        @update:model-value="(v) => { pw.next = v; pwErrors.next = null; }"
      />
      <FormField
        label="Confirm new password"
        type="password"
        :model-value="pw.confirm"
        autocomplete="new-password"
        :error="pwErrors.confirm"
        @update:model-value="(v) => { pw.confirm = v; pwErrors.confirm = null; }"
      />
    </div>
    <Btn variant="primary" :disabled="pwSaving" @click="updatePassword">Update password</Btn>

    <div class="atl-divider" />

    <div class="atl-sec-title">Appearance</div>
    <div class="flex items-center" style="gap: 14px;">
      <SegmentedControl
        :model-value="ui.theme"
        :options="THEME_OPTIONS"
        @update:model-value="selectTheme"
      />
      <span style="font-size: 12px; color: var(--c-muted);">Ayu Dark · default</span>
    </div>

    <div class="atl-divider" />

    <button type="button" class="atl-signout" @click="signOut">
      <Icon name="external-link" :size="14" />Sign out
    </button>
  </div>
</template>

<style scoped>
.atl-inline-field {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  max-width: 430px;
  margin-bottom: 22px;
}

/* The action sits beside an unlabelled field, so matching the input height keeps
   the row reading as a single control. Restricted to the direct child so the
   controls rendered inside the field itself are left alone. */
.atl-inline-field > :deep(button) {
  height: var(--h-input);
}

.atl-id-card {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 14px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: 4px;
  margin-bottom: 22px;
}

.atl-tag {
  font-size: 9.5px;
  font-weight: var(--fw-bold);
  letter-spacing: 0.06em;
  color: var(--c-muted);
  border: 1px solid var(--c-border);
  background: var(--c-background);
  border-radius: var(--r-sm);
  padding: 1px 5px;
  font-family: var(--font-mono);
}

.atl-tag-root {
  color: var(--c-primary);
  border-color: rgba(255, 180, 84, 0.45);
  background: rgba(255, 180, 84, 0.12);
}

.atl-sec-title {
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--c-muted);
  margin-bottom: 10px;
}

.atl-divider {
  height: 1px;
  background: var(--c-border);
  margin: 22px 0;
}

.atl-signout {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  height: 28px;
  padding: 0 12px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-danger);
  cursor: pointer;
  font-size: 12.5px;
}

.atl-signout:hover {
  background: var(--c-raised);
}
</style>
