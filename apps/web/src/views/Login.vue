<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue';
import { useRouter } from 'vue-router';
import { z } from 'zod';
import type { AtlasProblem } from '@/api/problem';
import Btn from '@/components/ui/Btn.vue';
import FormField from '@/components/ui/FormField.vue';
import Icon from '@/components/ui/Icon.vue';
import Kbd from '@/components/ui/Kbd.vue';
import { useProblem } from '@/composables/useProblem';
import { validateForm } from '@/lib/validation';
import { getPlatformTransport } from '@/platform/transport';
import { useAuthStore } from '@/stores/auth';

const loginSchema = z.object({
  origin: z.string().trim().min(1, 'Server URL is required'),
  username: z.string().trim().min(1, 'Username is required'),
  password: z.string().min(1, 'Password is required'),
});

const router = useRouter();
const auth = useAuthStore();
const transport = getPlatformTransport();

const origin = ref('https://atlas.iperez.dev');
const username = ref('');
const password = ref('');
const loading = ref(false);
const errorProblem = ref<AtlasProblem | null>(null);

const fieldErrors = reactive<{ origin: string | null; username: string | null; password: string | null }>({
  origin: null,
  username: null,
  password: null,
});

const FALLBACK_PROBLEM: AtlasProblem = {
  type: 'urn:atlas:error:unknown',
  title: 'Sign-in failed',
  status: 0,
  hint: 'Something went wrong signing in. Please try again.',
};

function onUsername(value: string) {
  username.value = value;
  fieldErrors.username = null;
}

function onOrigin(value: string) {
  origin.value = value;
  fieldErrors.origin = null;
}

function onPassword(value: string) {
  password.value = value;
  fieldErrors.password = null;
}

function validate(): boolean {
  const result = validateForm(loginSchema, {
    origin: origin.value,
    username: username.value,
    password: password.value,
  });

  fieldErrors.origin = result.ok ? null : (result.errors.origin ?? null);
  fieldErrors.username = result.ok ? null : (result.errors.username ?? null);
  fieldErrors.password = result.ok ? null : (result.errors.password ?? null);

  return result.ok;
}

async function handleLogin() {
  if (loading.value) return;

  errorProblem.value = null;
  if (!validate()) return;

  loading.value = true;

  try {
    if (transport.isDesktop) {
      const selected = await transport.setOrigin(origin.value);
      if (selected.error || selected.data === undefined) {
        fieldErrors.origin =
          typeof selected.error === 'string' ? selected.error : 'Unable to save the Atlas server URL';
        return;
      }
      origin.value = selected.data.origin;
    }

    const result = await auth.login({ username: username.value, password: password.value });

    if (result.ok) {
      const redirect = (router.currentRoute.value.query.redirect as string) ?? '/n';
      await router.replace(redirect);
      return;
    }

    errorProblem.value = (result.problem as AtlasProblem) ?? FALLBACK_PROBLEM;
  } catch {
    errorProblem.value = FALLBACK_PROBLEM;
  } finally {
    loading.value = false;
  }
}

onMounted(async () => {
  if (!transport.isDesktop) return;

  const selected = await transport.getOrigin();
  if (selected.data !== undefined) origin.value = selected.data.origin;
});

const errorDisplay = computed(() => {
  if (!errorProblem.value) return null;
  return useProblem(errorProblem.value);
});
</script>

<template>
  <div class="login-page">
    <div class="login-card">
      <div class="flex items-center" style="gap: 9px; margin-bottom: 18px;">
        <Icon name="atlas-glyph" :size="24" style="color: var(--c-primary);" />
        <span style="font-size: 19px; font-weight: 700; color: var(--c-foreground); font-family: var(--font-ui);">
          Atlas
        </span>
      </div>

      <div style="font-size: var(--fs-xl); font-weight: 700; color: var(--c-foreground); margin-bottom: 3px;">
        Sign in
      </div>
      <div style="font-size: var(--fs-base); color: var(--c-muted); margin-bottom: 18px;">
        Use your Atlas account
      </div>

      <div
        v-if="errorProblem"
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
          style="gap: 7px; font-size: var(--fs-sm); font-weight: 700; color: var(--c-banner-err-fg); margin-bottom: 3px;"
        >
          <Icon name="triangle-alert" :size="13" />
          Sign-in failed
        </div>
        <div style="font-size: var(--fs-sm); color: var(--c-banner-err-fg); opacity: 0.9;">
          {{ errorDisplay?.hint ?? errorDisplay?.message }}
        </div>
        <div
          v-if="errorDisplay?.requestId"
          style="font-size: var(--fs-xs); font-family: var(--font-mono); color: var(--c-banner-err-fg); opacity: 0.7; margin-top: 3px;"
        >
          trace {{ errorDisplay?.requestId }}
        </div>
      </div>

      <form novalidate @submit.prevent="handleLogin">
        <div v-if="transport.isDesktop" style="margin-bottom: 12px;">
          <FormField
            id="server-origin"
            label="Atlas server"
            type="text"
            :model-value="origin"
            autocomplete="url"
            placeholder="https://atlas.iperez.dev"
            mono
            :error="fieldErrors.origin"
            @update:model-value="onOrigin"
          />
        </div>

        <div style="margin-bottom: 12px;">
          <FormField
            id="username"
            label="Username"
            :model-value="username"
            autocomplete="username"
            placeholder="username"
            mono
            :error="fieldErrors.username"
            @update:model-value="onUsername"
          />
        </div>

        <div style="margin-bottom: 12px;">
          <FormField
            id="password"
            label="Password"
            type="password"
            :model-value="password"
            autocomplete="current-password"
            placeholder="password"
            mono
            :error="fieldErrors.password"
            @update:model-value="onPassword"
          />
        </div>

        <div style="height: 6px;" />

        <Btn
          variant="primary"
          type="submit"
          :disabled="loading"
          style="width: 100%; height: 34px; margin-bottom: 14px;"
        >
          Sign in
        </Btn>

        <div
          class="flex items-center justify-center"
          style="gap: 7px; font-size: var(--fs-sm); color: var(--c-muted);"
        >
          Press
          <Kbd label="↵" />
          to continue
        </div>
      </form>
    </div>
  </div>
</template>

<style scoped>
.login-page {
  display: flex;
  width: 100%;
  flex: 1 1 auto;
  min-height: 0;
  overflow: auto;
  padding: 16px;
  background-color: var(--c-background);
}

.login-card {
  width: 100%;
  min-width: 280px;
  max-width: 340px;
  flex-shrink: 0;
  margin: auto;
  padding: 26px 26px 22px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg);
  background-color: var(--c-panel);
  box-shadow: var(--shadow-lg);
}

@media (max-width: 311px) {
  .login-card {
    min-width: 0;
  }
}
</style>
