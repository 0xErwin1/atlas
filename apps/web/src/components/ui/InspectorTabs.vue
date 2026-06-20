<script setup lang="ts">
/**
 * Shared inspector/preview tab strip: a 36px header of text tabs with the active
 * one marked by a bold label and an inset primary underline. The single source
 * for every inspector tab bar (the editor inspector, the task inspector, the
 * search preview), so they can no longer drift apart.
 */

interface Tab {
  id: string;
  label: string;
}

defineProps<{
  tabs: Tab[];
}>();

const active = defineModel<string>('active', { required: true });
</script>

<template>
  <div class="atl-tabstrip">
    <button
      v-for="tab in tabs"
      :key="tab.id"
      type="button"
      class="atl-itab"
      :class="{ on: active === tab.id }"
      :aria-selected="active === tab.id"
      @click="active = tab.id"
    >
      {{ tab.label }}
    </button>
    <slot name="end" />
  </div>
</template>

<style scoped>
.atl-tabstrip {
  display: flex;
  align-items: flex-end;
  height: 36px;
  flex: 0 0 36px;
  padding: 0 4px;
  border-bottom: 1px solid var(--c-border);
}

.atl-itab {
  height: 28px;
  padding: 0 9px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: var(--fs-sm);
  font-weight: var(--fw-medium);
  color: var(--c-muted);
}

.atl-itab:hover {
  color: var(--c-foreground);
}

.atl-itab.on {
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
  box-shadow: inset 0 -2px 0 var(--c-primary);
}
</style>
