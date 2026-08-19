<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import { swatchById } from '@/lib/swatches';

export type ChipTone = 'info' | 'success' | 'warning' | 'danger' | 'agent' | 'neutral';

/** A tone is one token washed into the surface, drawn on, and outlined. */
function washOf(token: string): { bg: string; color: string; border: string } {
  return {
    bg: `color-mix(in srgb, var(${token}) 12%, transparent)`,
    color: `var(${token})`,
    border: `color-mix(in srgb, var(${token}) 40%, transparent)`,
  };
}

const TONE_STYLES: Record<ChipTone, { bg: string; color: string; border: string }> = {
  info: washOf('--c-info'),
  success: washOf('--c-success'),
  warning: washOf('--c-warning'),
  danger: washOf('--c-danger'),
  agent: washOf('--c-agent'),
  neutral: {
    bg: 'color-mix(in srgb, var(--c-foreground) 6%, transparent)',
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
      padding: '1px 6px',
      backgroundColor: style.bg,
      border: `1px solid ${style.border}`,
      color: style.color,
      fontFamily: 'var(--font-mono)',
      fontSize: 'var(--fs-label)',
      fontWeight: 'var(--fw-semibold)',
      lineHeight: '16px',
      whiteSpace: 'nowrap',
      ...(truncate ? { maxWidth: '100%', minWidth: '0', overflow: 'hidden' } : {}),
    }"
  >
    <Icon v-if="icon" :name="icon" :size="11" />
    <span
      v-if="truncate"
      style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;"
    >
      <slot />
    </span>
    <slot v-else />
  </span>
</template>
