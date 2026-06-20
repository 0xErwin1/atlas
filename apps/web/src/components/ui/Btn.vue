<script setup lang="ts">
export type BtnVariant = 'primary' | 'secondary' | 'ghost' | 'danger';

withDefaults(
  defineProps<{
    variant?: BtnVariant;
    disabled?: boolean;
    type?: 'button' | 'submit' | 'reset';
  }>(),
  {
    variant: 'secondary',
    disabled: false,
    type: 'button',
  },
);

defineEmits<{
  click: [event: MouseEvent];
}>();

const VARIANT_STYLES: Record<BtnVariant, string> = {
  primary:
    'background-color: var(--c-primary); color: var(--c-primary-fg); border: 1px solid transparent; font-weight: var(--fw-semibold);',
  secondary:
    'background-color: var(--c-secondary); color: var(--c-foreground); border: 1px solid var(--c-border); font-weight: var(--fw-medium);',
  ghost:
    'background-color: transparent; color: var(--c-foreground); border: 1px solid var(--c-border); font-weight: var(--fw-medium);',
  danger:
    'background-color: var(--c-danger); color: var(--c-danger-fg, #fff); border: 1px solid transparent; font-weight: var(--fw-semibold);',
};
</script>

<template>
  <button
    :type="type"
    :disabled="disabled"
    class="inline-flex items-center justify-center shrink-0 cursor-pointer select-none"
    :style="`
      height: var(--h-button);
      gap: 6px;
      padding: 0 12px;
      border-radius: var(--r-md);
      font-family: var(--font-mono);
      font-size: var(--fs-sm);
      font-weight: var(--fw-medium);
      line-height: 1;
      transition: background-color 0.1s;
      ${VARIANT_STYLES[variant]}
      ${disabled ? 'opacity: 0.45; cursor: not-allowed;' : ''}
    `"
    @click="!disabled && $emit('click', $event)"
  >
    <slot />
  </button>
</template>
