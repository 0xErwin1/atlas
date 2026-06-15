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
        padding: 32px;
        border-radius: var(--r-lg);
        background-color: var(--c-panel);
        border: 1px solid var(--c-border);
        box-shadow: var(--shadow-lg);
      "
    >
      <div class="flex flex-col items-center gap-4 mb-8">
        <Icon
          name="atlas-glyph"
          :size="44"
          style="color: var(--c-primary);"
        />
        <span style="font-family: var(--font-mono); font-size: var(--fs-lg); font-weight: var(--fw-semibold); color: var(--c-foreground);">
          Atlas
        </span>
      </div>

      <div
        v-if="errorProblem"
        style="
          margin-bottom: 16px;
          padding: 10px 12px;
          border-radius: var(--r-md);
          background-color: var(--c-banner-err-bg);
          border: 1px solid rgba(240, 113, 120, 0.35);
          color: var(--c-banner-err-fg);
          font-family: var(--font-mono);
          font-size: var(--fs-xs);
          line-height: var(--lh-normal);
        "
      >
        <span>{{ errorDisplay?.hint ?? errorDisplay?.message }}</span>
        <span
          v-if="errorDisplay?.requestId"
          style="display: block; opacity: 0.6; margin-top: 2px;"
        >
          trace {{ errorDisplay?.requestId }}
        </span>
      </div>

      <form class="flex flex-col gap-3" @submit.prevent="handleLogin">
        <div class="flex flex-col gap-1">
          <label
            for="username"
            style="font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--c-muted);"
          >
            Username
          </label>
          <input
            id="username"
            v-model="username"
            type="text"
            autocomplete="username"
            required
            :style="`
              height: var(--h-input);
              padding: 0 10px;
              border-radius: var(--r-md);
              background-color: var(--c-input);
              border: 1px solid ${errorProblem ? 'var(--c-danger)' : 'var(--c-border)'};
              color: var(--c-foreground);
              font-family: var(--font-mono);
              font-size: var(--fs-sm);
              outline: none;
              width: 100%;
            `"
          />
        </div>

        <div class="flex flex-col gap-1">
          <label
            for="password"
            style="font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--c-muted);"
          >
            Password
          </label>
          <div class="relative">
            <input
              id="password"
              v-model="password"
              :type="showPassword ? 'text' : 'password'"
              autocomplete="current-password"
              required
              :style="`
                height: var(--h-input);
                padding: 0 36px 0 10px;
                border-radius: var(--r-md);
                background-color: var(--c-input);
                border: 1px solid ${errorProblem ? 'var(--c-danger)' : 'var(--c-border)'};
                color: var(--c-foreground);
                font-family: var(--font-mono);
                font-size: var(--fs-sm);
                outline: none;
                width: 100%;
              `"
            />
            <button
              type="button"
              tabindex="-1"
              style="
                position: absolute;
                right: 8px;
                top: 50%;
                transform: translateY(-50%);
                color: var(--c-muted);
                background: none;
                border: none;
                cursor: pointer;
                padding: 0;
                display: flex;
                align-items: center;
              "
              @click="showPassword = !showPassword"
            >
              <Icon :name="showPassword ? 'eye-off' : 'eye'" :size="14" />
            </button>
          </div>
        </div>

        <Btn
          variant="primary"
          type="submit"
          :disabled="loading"
          class="mt-2 w-full justify-center gap-2"
        >
          <span>{{ loading ? 'Signing in…' : 'Sign in' }}</span>
          <Kbd label="↵" />
        </Btn>
      </form>
    </div>
  </div>
</template>
