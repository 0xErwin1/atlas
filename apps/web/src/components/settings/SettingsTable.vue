<script setup lang="ts">
// Bordered, rounded settings table shell with an uppercase muted header row.
// The `head` slot provides the header cells and the default slot provides the
// rows (typically ExpandableRow instances). Column widths are owned by the
// caller so the header cells and the row summary cells stay in sync.
</script>

<template>
  <div class="atl-settings-table">
    <div class="atl-settings-head">
      <slot name="head" />
    </div>
    <slot />
  </div>
</template>

<style scoped>
.atl-settings-table {
  border: 1px solid var(--c-border);
  border-radius: 4px;
  overflow: hidden;
}

.atl-settings-head {
  display: flex;
  align-items: center;
  height: 28px;
  padding: 0 12px;
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--c-muted);
}
</style>
