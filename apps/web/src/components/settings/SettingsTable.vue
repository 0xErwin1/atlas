<script setup lang="ts">
// Bordered settings table shell with an uppercase muted header row.
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
  border: 1px solid var(--c-border-strong);
  overflow: hidden;
}

/* The rule under the head is the heaviest line in the table: it separates the
   column names from the data, and every row rule below it is a single pixel. */
.atl-settings-table > .atl-settings-head + :deep(*) {
  border-top-width: var(--bw-head);
}

.atl-settings-head {
  display: flex;
  align-items: center;
  height: var(--h-header);
  padding: 0 12px;
  font-family: var(--font-mono);
  font-size: var(--fs-label);
  font-weight: var(--fw-semibold);
  letter-spacing: var(--ls-label);
  text-transform: uppercase;
  color: var(--c-muted);
}
</style>
