<script setup lang="ts">
withDefaults(
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
      fontSize: `${Math.max(10, Math.floor(size * 0.42))}px`,
      fontWeight: 600,
      color: agent ? 'var(--c-agent)' : 'var(--c-foreground)',
      lineHeight: '1',
    }"
  >
    <slot>{{ name ? name.slice(0, 2).toUpperCase() : '?' }}</slot>
    <slot name="sparkles" />
  </span>
</template>
