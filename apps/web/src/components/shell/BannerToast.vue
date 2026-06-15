<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import { useUiStore } from '@/stores/ui';

const ui = useUiStore();

const banner = computed(() => ui.banner);

const TONE_STYLES: Record<string, { bg: string; fg: string; border: string; icon: string }> = {
  error: {
    bg: 'var(--c-banner-err-bg)',
    fg: 'var(--c-banner-err-fg)',
    border: 'rgba(240, 113, 120, 0.35)',
    icon: 'alert-circle',
  },
  warning: {
    bg: 'rgba(255, 180, 84, 0.12)',
    fg: 'var(--c-warning)',
    border: 'rgba(255, 180, 84, 0.35)',
    icon: 'alert-triangle',
  },
  info: {
    bg: 'rgba(89, 194, 255, 0.12)',
    fg: 'var(--c-info)',
    border: 'rgba(89, 194, 255, 0.35)',
    icon: 'info',
  },
  success: {
    bg: 'rgba(170, 217, 76, 0.12)',
    fg: 'var(--c-success)',
    border: 'rgba(170, 217, 76, 0.35)',
    icon: 'check-circle',
  },
};
</script>

<template>
  <Transition name="banner">
    <div
      v-if="banner"
      class="fixed flex items-center gap-2"
      :style="`
        bottom: 16px;
        left: 50%;
        transform: translateX(-50%);
        min-width: 320px;
        max-width: 560px;
        padding: 10px 14px;
        border-radius: var(--r-md);
        background-color: ${TONE_STYLES[banner.type]?.bg ?? 'var(--c-raised)'};
        border: 1px solid ${TONE_STYLES[banner.type]?.border ?? 'var(--c-border)'};
        color: ${TONE_STYLES[banner.type]?.fg ?? 'var(--c-foreground)'};
        box-shadow: var(--shadow-lg);
        z-index: 9999;
        font-family: var(--font-mono);
        font-size: var(--fs-sm);
      `"
      role="alert"
      aria-live="polite"
    >
      <Icon
        :name="TONE_STYLES[banner.type]?.icon ?? 'info'"
        :size="16"
        style="flex-shrink: 0;"
      />

      <span class="flex-1">{{ banner.message }}</span>

      <button
        type="button"
        aria-label="Dismiss"
        style="
          background: none;
          border: none;
          cursor: pointer;
          color: inherit;
          opacity: 0.6;
          padding: 0;
          display: flex;
          align-items: center;
          flex-shrink: 0;
        "
        @click="ui.dismissBanner()"
      >
        <Icon name="x" :size="14" />
      </button>
    </div>
  </Transition>
</template>

<style scoped>
.banner-enter-active,
.banner-leave-active {
  transition: opacity 0.2s ease, transform 0.2s ease;
}

.banner-enter-from,
.banner-leave-to {
  opacity: 0;
  transform: translateX(-50%) translateY(8px);
}
</style>
