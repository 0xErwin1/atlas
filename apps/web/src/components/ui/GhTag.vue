<script setup lang="ts">
/**
 * GitHub-style label pill: a colored dot + rounded outline pill, matching the
 * hi-fi `GhTag`. Presentational; `active` adds a primary outline for use as a
 * toggleable filter chip.
 */

withDefaults(
  defineProps<{
    /** CSS color for the leading dot. */
    color?: string;
    active?: boolean;
  }>(),
  {
    color: 'var(--c-muted)',
    active: false,
  },
);
</script>

<template>
  <span class="atl-ghtag" :class="{ on: active }">
    <span class="atl-ghtag-dot" :style="{ background: color }" />
    <slot />
  </span>
</template>

<style scoped>
.atl-ghtag {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 2px 9px;
  border-radius: var(--r-full);
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  font-size: var(--fs-xs);
  font-weight: var(--fw-medium);
  color: var(--c-foreground);
  white-space: nowrap;
}

.atl-ghtag.on {
  border-color: var(--c-primary);
  background: var(--c-selection);
}

.atl-ghtag-dot {
  width: 8px;
  height: 8px;
  border-radius: var(--r-full);
  flex: 0 0 auto;
}
</style>
