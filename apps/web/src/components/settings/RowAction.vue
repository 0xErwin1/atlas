<script setup lang="ts">
withDefaults(
  defineProps<{
    tone?: 'default' | 'danger';
    /** Square icon-only variant (no label). */
    iconOnly?: boolean;
    disabled?: boolean;
    title?: string;
    type?: 'button' | 'submit' | 'reset';
  }>(),
  {
    tone: 'default',
    iconOnly: false,
    disabled: false,
    title: undefined,
    type: 'button',
  },
);

defineEmits<{
  click: [event: MouseEvent];
}>();
</script>

<template>
  <button
    :type="type"
    :title="title"
    :disabled="disabled"
    class="atl-rowact"
    :class="{ 'atl-rowact--icon': iconOnly, 'atl-rowact--danger': tone === 'danger' }"
    @click="!disabled && $emit('click', $event)"
  >
    <slot />
  </button>
</template>

<style scoped>
.atl-rowact {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  height: 26px;
  padding: 0 8px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-foreground);
  cursor: pointer;
  font-size: 12px;
}

.atl-rowact--icon {
  width: 28px;
  padding: 0;
}

.atl-rowact:hover:enabled {
  background: var(--c-raised);
}

.atl-rowact:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.atl-rowact--danger {
  color: var(--c-danger);
}
</style>
