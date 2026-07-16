<script setup lang="ts">
import Icon from '@/components/ui/Icon.vue';

export interface SegmentedOption {
  value: string;
  label: string;
  icon?: string;
}

defineProps<{
  modelValue: string;
  options: SegmentedOption[];
}>();

defineEmits<{
  'update:modelValue': [value: string];
}>();
</script>

<template>
  <div class="atl-seg">
    <button
      v-for="option in options"
      :key="option.value"
      type="button"
      class="atl-seg-opt"
      :class="{ on: modelValue === option.value }"
      @click="$emit('update:modelValue', option.value)"
    >
      <Icon v-if="option.icon" :name="option.icon" :size="13" />{{ option.label }}
    </button>
  </div>
</template>

<style scoped>
/* The global `.atl-seg` in theme/base.css is the editor-toolbar variant; these
   scoped rules deliberately re-specify it for the settings variant and pair it
   with `.atl-seg-opt`, which exists nowhere else. */
.atl-seg {
  display: inline-flex;
  background: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg);
  padding: 2px;
  gap: 2px;
}

.atl-seg-opt {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 26px;
  padding: 0 13px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  cursor: pointer;
  font-size: 12.5px;
  font-weight: var(--fw-medium);
  color: var(--c-muted);
}

.atl-seg-opt.on {
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
  background: var(--c-selection);
  box-shadow: inset 0 0 0 1px var(--c-border);
}
</style>
