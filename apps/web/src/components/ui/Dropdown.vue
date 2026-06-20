<script setup lang="ts">
import Icon from '@/components/ui/Icon.vue';
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
    /** Optional leading icon rendered before the label. */
    icon?: string;
  }>(),
  {
    modelValue: undefined,
    placeholder: 'Select…',
    disabled: false,
    icon: '',
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
        class="inline-flex items-center cursor-pointer select-none"
        :disabled="disabled"
        :style="`
          height: var(--h-button);
          gap: 7px;
          padding: 0 9px;
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
        <Icon v-if="icon" :name="icon" :size="13" style="color: var(--c-muted); flex: 0 0 auto;" />
        <span>{{ selectedLabel() }}</span>
        <Icon
          name="chevron-down"
          :size="12"
          :style="{
            flex: '0 0 auto',
            color: 'var(--c-muted)',
            transform: open ? 'rotate(180deg)' : 'none',
            transition: 'transform 0.1s',
          }"
        />
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
