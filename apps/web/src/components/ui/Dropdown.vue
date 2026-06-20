<script setup lang="ts">
import Popover from '@/components/ui/Popover.vue';

export interface DropdownOption {
  value: string;
  label: string;
  disabled?: boolean;
}

const props = withDefaults(
  defineProps<{
    options: DropdownOption[];
    modelValue?: string;
    placeholder?: string;
    disabled?: boolean;
  }>(),
  {
    modelValue: undefined,
    placeholder: 'Select…',
    disabled: false,
  },
);

const emit = defineEmits<{
  'update:modelValue': [value: string];
  change: [value: string];
}>();

function selectOption(opt: DropdownOption): void {
  if (opt.disabled) return;
  emit('update:modelValue', opt.value);
  emit('change', opt.value);
}

const selectedLabel = (): string => {
  const found = props.options.find((o) => o.value === props.modelValue);
  return found ? found.label : props.placeholder;
};
</script>

<template>
  <Popover placement="bottom-start">
    <template #trigger="{ open, toggle }">
      <button
        type="button"
        class="inline-flex items-center gap-1 cursor-pointer select-none"
        :disabled="disabled"
        :style="`
          height: var(--h-button);
          padding: 0 8px;
          border-radius: var(--r-md);
          background-color: var(--c-raised);
          border: 1px solid var(--c-border);
          color: var(--c-foreground);
          font-family: var(--font-mono);
          font-size: var(--fs-sm);
          font-weight: var(--fw-medium);
          ${disabled ? 'opacity: 0.45; cursor: not-allowed;' : ''}
        `"
        @click="toggle"
      >
        <span>{{ selectedLabel() }}</span>
        <svg
          width="12"
          height="12"
          viewBox="0 0 12 12"
          fill="none"
          aria-hidden="true"
          :style="{ transform: open ? 'rotate(180deg)' : 'none', transition: 'transform 0.1s' }"
        >
          <path d="M3 4.5L6 7.5L9 4.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
        </svg>
      </button>
    </template>

    <template #default="{ close }">
      <ul role="listbox" style="list-style: none; padding: 2px 0; min-width: 100%;">
        <li
          v-for="opt in options"
          :key="opt.value"
          role="option"
          :aria-selected="opt.value === modelValue"
          :aria-disabled="opt.disabled"
          class="flex items-center px-3 cursor-pointer"
          :style="`
            height: var(--h-compact);
            white-space: nowrap;
            font-size: var(--fs-sm);
            font-family: var(--font-mono);
            ${opt.disabled ? 'opacity: 0.45; cursor: not-allowed;' : ''}
            ${opt.value === modelValue ? 'background-color: var(--c-selection); color: var(--c-foreground);' : 'color: var(--c-foreground);'}
          `"
          @click="selectOption(opt), !opt.disabled && close()"
        >
          {{ opt.label }}
        </li>
      </ul>
    </template>
  </Popover>
</template>
