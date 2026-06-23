<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { z } from 'zod';
import { wrappedClient } from '@/api/wrapper';
import Btn from '@/components/ui/Btn.vue';
import FormField from '@/components/ui/FormField.vue';
import Icon from '@/components/ui/Icon.vue';
import { validateForm } from '@/lib/validation';
import { useAuthStore } from '@/stores/auth';
import { useWorkspaceStore } from '@/stores/workspace';

const route = useRoute();
const router = useRouter();
const auth = useAuthStore();
const wsStore = useWorkspaceStore();

const token = computed(() => {
  const value = route.params.token;
  return typeof value === 'string' ? value : '';
});

type Phase = 'loading' | 'ready' | 'invalid';
const phase = ref<Phase>('loading');

const account = ref<{ username: string; display_name: string } | null>(null);

const form = reactive({ password: '', confirm: '' });
const fieldErrors = reactive<{ password: string | null; confirm: string | null }>({
  password: null,
  confirm: null,
});
const submitting = ref(false);
const submitError = ref<string | null>(null);

const INVALID_MESSAGE = 'This link is invalid or has already been used.';

const activateSchema = z
  .object({
    password: z.string().min(8, 'Use at least 8 characters'),
    confirm: z.string().min(1, 'Repeat the password'),
  })
  .refine((v) => v.password === v.confirm, {
    path: ['confirm'],
    message: "Passwords don't match",
  });

onMounted(async () => {
  if (token.value === '') {
    phase.value = 'invalid';
    return;
  }

  try {
    const { data, error } = await wrappedClient.GET('/v1/activate/{token}', {
      params: { path: { token: token.value } },
    });

    if (error || !data) {
      phase.value = 'invalid';
      return;
    }

    account.value = data;
    phase.value = 'ready';
  } catch {
    phase.value = 'invalid';
  }
});

function onPassword(value: string): void {
  form.password = value;
  fieldErrors.password = null;
  submitError.value = null;
}

function onConfirm(value: string): void {
  form.confirm = value;
  fieldErrors.confirm = null;
  submitError.value = null;
}

interface SubmitProblem {
  status?: number;
  hint?: string;
  title?: string;
}

function submitErrorMessage(error: SubmitProblem | undefined): string {
  const status = error?.status;

  if (status === 404) return INVALID_MESSAGE;
  if (status === 429) return 'Too many attempts. Wait a moment and try again.';
  if (status === 422) return error?.hint ?? "The password doesn't meet the requirements.";

  return error?.hint ?? error?.title ?? "Couldn't activate the account. Try again.";
}

async function handleSubmit(): Promise<void> {
  if (submitting.value) return;

  submitError.value = null;

  const result = validateForm(activateSchema, { password: form.password, confirm: form.confirm });
  fieldErrors.password = result.ok ? null : (result.errors.password ?? null);
  fieldErrors.confirm = result.ok ? null : (result.errors.confirm ?? null);
  if (!result.ok) return;

  submitting.value = true;

  try {
    const { error } = await wrappedClient.POST('/v1/activate/{token}', {
      params: { path: { token: token.value } },
      body: { password: result.data.password },
    });

    if (error) {
      const problem = error as SubmitProblem;
      if (problem.status === 404) {
        phase.value = 'invalid';
        return;
      }
      submitError.value = submitErrorMessage(problem);
      return;
    }

    // The POST set the session cookie (auto-login). Hydrate auth + workspaces,
    // then land the user inside the app.
    await auth.fetchMe();
    await wsStore.loadWorkspaces();
    await router.replace('/n');
  } catch {
    submitError.value = "Couldn't reach the server. Try again.";
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <div
    class="flex items-center justify-center"
    style="min-height: 100vh; background-color: var(--c-background);"
  >
    <div
      style="
        width: 340px;
        padding: 26px 26px 22px;
        border-radius: var(--r-lg);
        background-color: var(--c-panel);
        border: 1px solid var(--c-border);
        box-shadow: var(--shadow-lg);
      "
    >
      <div class="flex items-center" style="gap: 9px; margin-bottom: 18px;">
        <Icon name="atlas-glyph" :size="24" style="color: var(--c-primary);" />
        <span style="font-size: 19px; font-weight: 700; color: var(--c-foreground); font-family: var(--font-ui);">
          Atlas
        </span>
      </div>

      <!-- Loading -->
      <div v-if="phase === 'loading'" style="font-size: var(--fs-base); color: var(--c-muted); padding: 8px 0;">
        Verifying the link…
      </div>

      <!-- Invalid / expired / consumed -->
      <div v-else-if="phase === 'invalid'">
        <div
          style="font-size: var(--fs-xl); font-weight: 700; color: var(--c-foreground); margin-bottom: 3px;"
        >
          Invalid link
        </div>
        <div style="font-size: var(--fs-base); color: var(--c-muted); margin-bottom: 18px;">
          {{ INVALID_MESSAGE }}
        </div>
        <div style="font-size: var(--fs-sm); color: var(--c-muted); margin-bottom: 18px; line-height: 1.5;">
          Ask whoever invited you to generate a new activation link.
        </div>
        <Btn
          variant="secondary"
          style="width: 100%; height: 34px;"
          @click="router.replace('/login')"
        >
          Go to sign in
        </Btn>
      </div>

      <!-- Ready: set password -->
      <div v-else-if="phase === 'ready' && account">
        <div
          style="font-size: var(--fs-xl); font-weight: 700; color: var(--c-foreground); margin-bottom: 3px;"
        >
          Activate your account
        </div>
        <div style="font-size: var(--fs-base); color: var(--c-muted); margin-bottom: 18px;">
          Hi
          <span style="color: var(--c-foreground); font-weight: 600;">{{ account.display_name }}</span>
          <span style="font-family: var(--font-mono); color: var(--c-muted);"> (@{{ account.username }})</span>.
          Choose a password to finish.
        </div>

        <div
          v-if="submitError"
          style="
            background-color: var(--c-banner-err-bg);
            border: 1px solid rgba(240, 113, 120, 0.5);
            border-radius: var(--r-md);
            padding: 9px 11px;
            margin-bottom: 14px;
          "
        >
          <div
            class="flex items-center"
            style="gap: 7px; font-size: var(--fs-sm); color: var(--c-banner-err-fg); opacity: 0.95;"
          >
            <Icon name="triangle-alert" :size="13" />
            {{ submitError }}
          </div>
        </div>

        <form novalidate @submit.prevent="handleSubmit">
          <div style="margin-bottom: 12px;">
            <FormField
              id="password"
              label="Password"
              type="password"
              :model-value="form.password"
              autocomplete="new-password"
              placeholder="password"
              helper="At least 8 characters."
              :error="fieldErrors.password"
              @update:model-value="onPassword"
            />
          </div>

          <div style="margin-bottom: 12px;">
            <FormField
              id="confirm"
              label="Confirm password"
              type="password"
              :model-value="form.confirm"
              autocomplete="new-password"
              placeholder="password"
              :error="fieldErrors.confirm"
              @update:model-value="onConfirm"
            />
          </div>

          <div style="height: 6px;" />

          <Btn
            variant="primary"
            type="submit"
            :disabled="submitting"
            style="width: 100%; height: 34px;"
          >
            Activate and sign in
          </Btn>
        </form>
      </div>
    </div>
  </div>
</template>
