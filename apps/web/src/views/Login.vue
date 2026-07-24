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
import { APP_COMMIT, APP_VERSION } from '@/lib/buildInfo';
import { validateForm } from '@/lib/validation';
import { fetchThroughPlatform } from '@/platform/fetch';
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

const origin = ref('');
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

const errorDisplay = computed(() => {
  if (!errorProblem.value) return null;
  return useProblem(errorProblem.value);
});

/**
 * Parsed view of the currently selected server, used by the status bar. Guards
 * against an empty or unparseable origin so the footer never throws while the
 * origin is still being hydrated or the user is mid-edit on desktop.
 */
const serverInfo = computed<{ secure: boolean; host: string } | null>(() => {
  const raw = origin.value.trim();
  if (!raw) return null;

  try {
    const url = new URL(raw);
    return { secure: url.protocol === 'https:', host: url.host };
  } catch {
    return null;
  }
});

type HealthStatus = 'connecting' | 'ready' | 'offline';

const health = ref<HealthStatus>('connecting');

const healthColor = computed(() => {
  switch (health.value) {
    case 'ready':
      return 'var(--c-success)';
    case 'offline':
      return 'var(--c-danger)';
    default:
      return 'var(--c-muted)';
  }
});

const healthLabel = computed(() => {
  switch (health.value) {
    case 'ready':
      return 'ready';
    case 'offline':
      return 'offline';
    default:
      return 'connecting…';
  }
});

/**
 * Probes the public, unauthenticated `GET /health` endpoint once to reflect real
 * server reachability in the status bar. Routing goes through the platform fetch
 * so it reaches the configured server on both browser and desktop; the URL host is
 * only a base for the relative path and is ignored by the desktop transport.
 */
async function probeHealth(): Promise<void> {
  try {
    const base = globalThis.location?.origin ?? 'http://localhost';
    const response = await fetchThroughPlatform(new Request(new URL('/health', base), { method: 'GET' }));
    health.value = response.ok ? 'ready' : 'offline';
  } catch {
    health.value = 'offline';
  }
}

onMounted(async () => {
  const selected = await transport.getOrigin();
  if (selected.data !== undefined) origin.value = selected.data.origin;

  void probeHealth();
});
</script>

<template>
  <div class="login-page">
    <header class="login-topbar">
      <div class="login-brand">
        <Icon name="atlas-glyph" :size="20" style="color: var(--c-primary);" />
        <span class="login-wordmark">Atlas</span>
      </div>

      <span class="login-version">v{{ APP_VERSION }} · {{ APP_COMMIT }}</span>
    </header>

    <main class="login-main">
      <div class="login-col">
        <h1 class="login-heading">Sign in</h1>
        <p class="login-subtitle">Use your Atlas account to continue</p>

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
          <div v-if="transport.isDesktop" class="login-field">
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

          <div class="login-field">
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

          <div class="login-field">
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

          <Btn
            variant="primary"
            type="submit"
            :disabled="loading"
            style="width: 100%; height: 34px; margin-top: 4px; margin-bottom: 14px;"
          >
            Sign in
          </Btn>

          <div class="login-hint">
            Press
            <Kbd label="↵" />
            to continue
          </div>
        </form>
      </div>
    </main>

    <footer class="login-statusbar">
      <div class="login-status-group">
        <template v-if="serverInfo">
          <span class="login-status-item">
            <Icon :name="serverInfo.secure ? 'lock' : 'unlock'" :size="13" />
            {{ serverInfo.secure ? 'https' : 'http' }}
          </span>
          <span class="login-status-item">
            <Icon name="globe" :size="13" />
            {{ serverInfo.host }}
          </span>
        </template>
        <span v-else class="login-status-item">
          <Icon name="globe" :size="13" />
          —
        </span>
      </div>

      <div class="login-status-group">
        <span class="login-status-item">
          <Icon name="dot" :size="12" :style="{ color: healthColor }" />
          {{ healthLabel }}
        </span>
      </div>
    </footer>
  </div>
</template>

<style scoped>
.login-page {
  display: flex;
  flex-direction: column;
  min-height: 0;
  flex: 1 1 auto;
  background: var(--c-background);
  font-family: var(--font-mono);
}

.login-topbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 18px;
  border-bottom: 1px solid var(--c-border);
}

.login-brand {
  display: flex;
  align-items: center;
  gap: 8px;
}

.login-wordmark {
  font-size: 14px;
  font-weight: 700;
  color: var(--c-foreground);
}

.login-version {
  font-size: 11px;
  color: var(--c-muted);
}

.login-main {
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 0;
  overflow: auto;
  padding: 24px;
}

.login-col {
  width: 100%;
  max-width: 380px;
}

.login-heading {
  font-size: 24px;
  font-weight: 700;
  color: var(--c-foreground);
  margin-bottom: 4px;
}

.login-subtitle {
  font-size: var(--fs-base);
  color: var(--c-muted);
  margin-bottom: 20px;
}

.login-field {
  margin-bottom: 14px;
}

.login-hint {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: var(--fs-sm);
  color: var(--c-muted);
}

.login-statusbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 8px 18px;
  border-top: 1px solid var(--c-border);
  font-size: 11px;
  color: var(--c-muted);
}

.login-status-group {
  display: flex;
  align-items: center;
  gap: 14px;
}

.login-status-item {
  display: flex;
  align-items: center;
  gap: 5px;
}
</style>
