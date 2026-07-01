<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import { swatchById } from '@/lib/swatches';

export type ChipTone = 'info' | 'success' | 'warning' | 'danger' | 'agent' | 'neutral';

const TONE_STYLES: Record<ChipTone, { bg: string; color: string; border: string }> = {
  info: { bg: 'rgba(89, 194, 255, 0.12)', color: 'var(--c-info)', border: 'rgba(89, 194, 255, 0.4)' },
  success: { bg: 'rgba(170, 217, 76, 0.12)', color: 'var(--c-success)', border: 'rgba(170, 217, 76, 0.4)' },
  warning: { bg: 'rgba(255, 180, 84, 0.12)', color: 'var(--c-primary)', border: 'rgba(255, 180, 84, 0.4)' },
  danger: { bg: 'rgba(240, 113, 120, 0.12)', color: 'var(--c-danger)', border: 'rgba(240, 113, 120, 0.4)' },
  agent: { bg: 'var(--c-agent-bg)', color: 'var(--c-agent)', border: 'var(--c-agent-border)' },
  neutral: {
    bg: 'rgba(179, 177, 173, 0.06)',
    color: 'var(--c-foreground)',
    border: 'var(--c-border)',
  },
};

const props = withDefaults(
  defineProps<{
    tone?: ChipTone;
    icon?: string;
    /** A user-chosen swatch id (see lib/swatches). Overrides `tone` when set. */
    color?: string;
    /** Cap the chip at its container's width and ellipsize a long label instead of
     * letting it overflow. For width-constrained hosts (task cards, table cells)
     * where a single long tag would otherwise stick out of the container. */
    truncate?: boolean;
  }>(),
  {
    tone: 'neutral',
    icon: '',
    color: '',
    truncate: false,
  },
);

// An explicit user-picked color wins over the semantic tone.
const style = computed(() => {
  if (props.color !== '') {
    const swatch = swatchById(props.color);
    return { bg: swatch.bg, color: swatch.fg, border: swatch.border };
  }
  return TONE_STYLES[props.tone];
});
</script>

<template>
  <span
    class="inline-flex items-center shrink-0 select-none"
    :style="{
      gap: '5px',
      padding: '3px 9px',
      borderRadius: 'var(--r-sm)',
      backgroundColor: style.bg,
      border: `1px solid ${style.border}`,
      color: style.color,
      fontFamily: 'var(--font-mono)',
      fontSize: 'var(--fs-base)',
      fontWeight: 'var(--fw-medium)',
      lineHeight: '1',
      whiteSpace: 'nowrap',
      ...(truncate ? { maxWidth: '100%', minWidth: '0', overflow: 'hidden' } : {}),
    }"
  >
    <Icon v-if="icon" :name="icon" :size="13" />
    <span
      v-if="truncate"
      style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;"
    >
      <slot />
    </span>
    <slot v-else />
  </span>
</template>
