<script setup lang="ts">
import Icon from '@/components/ui/Icon.vue';

withDefaults(
  defineProps<{
    title: string;
    hint?: string;
    icon?: string;
    /** Wrap the content in the design's bordered card with a 32px context bar. */
    framed?: boolean;
    /** Mono context label shown in the framed card's top bar (e.g. "Atlas · Notes"). */
    topLabel?: string;
  }>(),
  {
    hint: undefined,
    icon: 'file',
    framed: false,
    topLabel: undefined,
  },
);
</script>

<template>
  <div
    v-if="framed"
    data-state="empty"
    class="flex flex-col"
    style="width: 100%; height: 100%; background-color: var(--c-background); border: 1px solid var(--c-border); border-radius: 3px; overflow: hidden;"
  >
    <div
      class="flex items-center"
      style="gap: 8px; height: 32px; flex: 0 0 32px; padding: 0 10px; border-bottom: 1px solid var(--c-border); background-color: var(--c-panel);"
    >
      <button
        type="button"
        class="atl-gbtn"
        style="width: 22px; height: 22px;"
        aria-label="Toggle panel"
      >
        <Icon name="panel-right" :size="13" />
      </button>
      <span
        v-if="topLabel"
        style="font-size: var(--fs-sm); color: var(--c-muted); font-family: var(--font-mono);"
      >
        {{ topLabel }}
      </span>
    </div>

    <div
      class="flex flex-col items-center justify-center text-center"
      style="flex: 1; gap: 10px; padding: 24px; min-height: 0;"
    >
      <Icon :name="icon" :size="26" :style="{ color: 'var(--c-muted)' }" />
      <div style="font-size: 17px; font-weight: var(--fw-bold); color: var(--c-foreground);">
        {{ title }}
      </div>
      <div v-if="hint" style="font-size: var(--fs-base); color: var(--c-muted);">
        {{ hint }}
      </div>
      <div v-if="$slots.actions" class="flex" style="gap: 8px; margin-top: 4px;">
        <slot name="actions" />
      </div>
    </div>
  </div>

  <div
    v-else
    data-state="empty"
    class="flex flex-col items-center justify-center text-center"
    style="gap: 10px; padding: 24px; flex: 1; min-height: 0;"
  >
    <Icon :name="icon" :size="26" :style="{ color: 'var(--c-muted)' }" />
    <div style="font-size: 17px; font-weight: var(--fw-bold); color: var(--c-foreground);">
      {{ title }}
    </div>
    <div
      v-if="hint"
      style="font-size: var(--fs-base); color: var(--c-muted);"
    >
      {{ hint }}
    </div>
    <div
      v-if="$slots.actions"
      class="flex"
      style="gap: 8px; margin-top: 4px;"
    >
      <slot name="actions" />
    </div>
  </div>
</template>
