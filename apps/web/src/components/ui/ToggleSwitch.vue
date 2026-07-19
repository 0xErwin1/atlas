<script setup lang="ts">
/**
 * Controlled on/off switch with an adjacent label and help copy. The parent owns
 * the state: clicking emits `update:modelValue` with the negated value but never
 * flips anything internally, so a call site can either bind it as a plain
 * `v-model` toggle or intercept the change to run an async request or a
 * confirmation first. Extracted from the identical "Global agent" / "System
 * admin" switches that ApiKeysPanel and UsersPanel used to hand-roll.
 */
withDefaults(
  defineProps<{
    modelValue: boolean;
    label: string;
    copy?: string;
    disabled?: boolean;
  }>(),
  {
    copy: '',
    disabled: false,
  },
);

const emit = defineEmits<{ 'update:modelValue': [value: boolean] }>();

defineOptions({ inheritAttrs: false });

function onClick(current: boolean, disabled: boolean): void {
  if (disabled) return;
  emit('update:modelValue', !current);
}
</script>

<template>
  <div class="atl-switch-row">
    <button
      v-bind="$attrs"
      type="button"
      role="switch"
      class="atl-switch"
      :class="{ 'atl-switch--on': modelValue }"
      :aria-checked="modelValue"
      :disabled="disabled"
      data-switch
      @click="onClick(modelValue, disabled)"
    >
      <span class="atl-switch-knob" />
    </button>
    <div class="atl-switch-copy">
      <div class="atl-switch-label">{{ label }}</div>
      <div v-if="copy" class="atl-switch-help">{{ copy }}</div>
    </div>
  </div>
</template>

<style scoped>
.atl-switch-row {
  display: flex;
  align-items: flex-start;
  gap: 10px;
}

.atl-switch {
  flex: 0 0 auto;
  position: relative;
  width: 34px;
  height: 20px;
  margin-top: 1px;
  padding: 0;
  border: 1px solid var(--c-border);
  border-radius: 9999px;
  background: var(--c-input);
  cursor: pointer;
  transition: background 0.15s, border-color 0.15s;
}

.atl-switch--on {
  background: var(--c-agent);
  border-color: var(--c-agent);
}

.atl-switch:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.atl-switch-knob {
  position: absolute;
  top: 50%;
  left: 2px;
  width: 14px;
  height: 14px;
  border-radius: 9999px;
  background: var(--c-foreground);
  transform: translateY(-50%);
  transition: left 0.15s;
}

.atl-switch--on .atl-switch-knob {
  left: 17px;
  background: var(--c-on-agent, #fff);
}

.atl-switch-copy {
  min-width: 0;
}

.atl-switch-label {
  font-size: 12.5px;
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-switch-help {
  font-size: 11.5px;
  color: var(--c-muted);
  line-height: 1.45;
  margin-top: 2px;
  max-width: 440px;
}
</style>
