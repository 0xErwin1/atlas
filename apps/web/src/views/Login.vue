<script setup lang="ts">
import { computed, ref } from 'vue';
import { useRouter } from 'vue-router';
import type { AtlasProblem } from '@/api/problem';
import Btn from '@/components/ui/Btn.vue';
import Icon from '@/components/ui/Icon.vue';
import Kbd from '@/components/ui/Kbd.vue';
import { useProblem } from '@/composables/useProblem';
import { useAuthStore } from '@/stores/auth';

const router = useRouter();
const auth = useAuthStore();

const username = ref('');
const password = ref('');
const showPassword = ref(false);
const loading = ref(false);
const errorProblem = ref<AtlasProblem | null>(null);

async function handleLogin() {
  if (loading.value) return;

  loading.value = true;
  errorProblem.value = null;

  const result = await auth.login({ username: username.value, password: password.value });

  loading.value = false;

  if (result.ok) {
    const redirect = (router.currentRoute.value.query.redirect as string) ?? '/n';
    await router.replace(redirect);
    return;
  }

  if (result.problem) {
    errorProblem.value = result.problem as AtlasProblem;
  }
}

const errorDisplay = computed(() => {
  if (!errorProblem.value) return null;
  return useProblem(errorProblem.value);
});
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

      <form @submit.prevent="handleLogin">
        <div>
          <label
            for="username"
            style="display: block; font-size: 10px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; color: var(--c-muted); margin-bottom: 5px;"
          >
            Username
          </label>
          <div
            class="flex items-center"
            :style="`
              gap: 8px;
              height: var(--h-input);
              padding: 0 10px;
              margin-bottom: 12px;
              border-radius: var(--r-md);
              background-color: var(--c-input);
              border: 1px solid ${errorProblem ? 'var(--c-danger)' : 'var(--c-border)'};
            `"
          >
            <input
              id="username"
              v-model="username"
              type="text"
              autocomplete="username"
              placeholder="username"
              required
              class="atl-login-input"
            />
          </div>
        </div>

        <div>
          <label
            for="password"
            style="display: block; font-size: 10px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; color: var(--c-muted); margin-bottom: 5px;"
          >
            Password
          </label>
          <div
            class="flex items-center"
            :style="`
              gap: 8px;
              height: var(--h-input);
              padding: 0 10px;
              margin-bottom: 12px;
              border-radius: var(--r-md);
              background-color: var(--c-input);
              border: 1px solid ${errorProblem ? 'var(--c-danger)' : 'var(--c-border)'};
            `"
          >
            <input
              id="password"
              v-model="password"
              :type="showPassword ? 'text' : 'password'"
              autocomplete="current-password"
              placeholder="password"
              required
              class="atl-login-input"
            />
            <button
              type="button"
              tabindex="-1"
              class="flex items-center"
              style="color: var(--c-muted); background: none; border: none; cursor: pointer; padding: 0;"
              @click="showPassword = !showPassword"
            >
              <Icon :name="showPassword ? 'eye-off' : 'eye'" :size="14" />
            </button>
          </div>
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
.atl-login-input {
  flex: 1;
  min-width: 0;
  background: transparent;
  border: none;
  outline: none;
  color: var(--c-foreground);
  font-family: var(--font-mono);
  font-size: var(--fs-base);
}

.atl-login-input::placeholder {
  color: var(--c-muted);
}
</style>
