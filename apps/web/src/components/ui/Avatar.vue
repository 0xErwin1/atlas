<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';

const props = withDefaults(
  defineProps<{
    name?: string;
    size?: number;
    agent?: boolean;
  }>(),
  {
    name: '',
    size: 26,
    agent: false,
  },
);

// Small avatars (the design's range) use a fixed 9px; larger ones the app adds
// (account, reset) scale proportionally so initials stay readable.
const fontSize = computed(() => (props.size <= 18 ? 9 : Math.max(10, Math.floor(props.size * 0.42))));
const sparkleSize = computed(() => (props.size <= 18 ? 11 : 13));
</script>

<template>
  <span
    class="relative inline-flex items-center justify-center shrink-0 overflow-hidden select-none"
    :style="{
      width: `${size}px`,
      height: `${size}px`,
      borderRadius: '2px',
      backgroundColor: agent ? 'var(--c-agent-bg)' : 'var(--c-raised)',
      border: agent ? '1px solid var(--c-agent-border)' : '1px solid var(--c-border)',
      fontFamily: 'var(--font-mono)',
      fontSize: `${fontSize}px`,
      fontWeight: 700,
      color: agent ? 'var(--c-agent)' : 'var(--c-foreground)',
      lineHeight: '1',
    }"
  >
    <Icon v-if="agent" name="sparkles" :size="sparkleSize" />
    <slot v-else>{{ name ? name.slice(0, 2).toUpperCase() : '?' }}</slot>
  </span>
</template>
