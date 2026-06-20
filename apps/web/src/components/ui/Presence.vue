<script setup lang="ts">
/**
 * Agent presence pill: a small agent-toned label with a leading pulsing dot,
 * used to signal live agent activity (e.g. "Claude working"). The label comes
 * from the default slot.
 */
</script>

<template>
  <span
    class="inline-flex items-center select-none"
    :style="{
      gap: '6px',
      padding: '2px 8px',
      borderRadius: 'var(--r-md)',
      color: 'var(--c-agent)',
      background: 'var(--c-agent-bg)',
      border: '1px solid var(--c-agent-border)',
      fontFamily: 'var(--font-mono)',
      fontSize: '11px',
      fontWeight: 'var(--fw-medium)',
      whiteSpace: 'nowrap',
    }"
  >
    <span
      class="atl-pulse shrink-0"
      :style="{
        width: '6px',
        height: '6px',
        borderRadius: 'var(--r-full)',
        background: 'var(--c-agent)',
      }"
    />
    <slot />
  </span>
</template>
