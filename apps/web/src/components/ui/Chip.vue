<script setup lang="ts">
export type ChipTone = 'info' | 'success' | 'warning' | 'danger' | 'agent' | 'neutral';

const TONE_STYLES: Record<ChipTone, { bg: string; color: string; border: string }> = {
  info: { bg: 'rgba(89, 194, 255, 0.12)', color: 'var(--c-info)', border: 'rgba(89, 194, 255, 0.35)' },
  success: { bg: 'rgba(170, 217, 76, 0.12)', color: 'var(--c-success)', border: 'rgba(170, 217, 76, 0.35)' },
  warning: { bg: 'rgba(255, 180, 84, 0.12)', color: 'var(--c-warning)', border: 'rgba(255, 180, 84, 0.35)' },
  danger: { bg: 'rgba(240, 113, 120, 0.12)', color: 'var(--c-danger)', border: 'rgba(240, 113, 120, 0.35)' },
  agent: { bg: 'var(--c-agent-bg)', color: 'var(--c-agent)', border: 'var(--c-agent-border)' },
  neutral: {
    bg: 'var(--c-raised)',
    color: 'var(--c-foreground)',
    border: 'var(--c-border)',
  },
};

const props = withDefaults(
  defineProps<{
    tone?: ChipTone;
  }>(),
  {
    tone: 'neutral',
  },
);

const style = TONE_STYLES[props.tone];
</script>

<template>
  <span
    class="inline-flex items-center shrink-0 select-none"
    :style="{
      padding: '1px 6px',
      borderRadius: 'var(--r-full)',
      backgroundColor: style.bg,
      border: `1px solid ${style.border}`,
      color: style.color,
      fontFamily: 'var(--font-mono)',
      fontSize: 'var(--fs-xs)',
      fontWeight: 'var(--fw-medium)',
      lineHeight: '1.4',
      whiteSpace: 'nowrap',
    }"
  >
    <slot />
  </span>
</template>
