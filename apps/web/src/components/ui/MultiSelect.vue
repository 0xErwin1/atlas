<script setup lang="ts">
/**
 * Multi-select dropdown built on the shared Popover primitive. Selected values
 * render as inline removable chips inside the trigger; the popover panel lists
 * every option with a checkbox and an optional status dot. Selecting a row
 * toggles the value without closing the panel, so several can be chosen in one
 * pass.
 */
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';

export interface MultiSelectOption {
  value: string;
  label: string;
  /** A CSS color or token for a leading status dot. */
  dot?: string;
  /** A leading icon, as an alternative to `dot`. */
  icon?: string;
  /** When true the option is rendered visibly but cannot be toggled. */
  disabled?: boolean;
}

const props = withDefaults(
  defineProps<{
    options: MultiSelectOption[];
    placeholder?: string;
    icon?: string;
  }>(),
  {
    placeholder: 'Any',
    icon: '',
  },
);

const model = defineModel<string[]>({ required: true });

function isSelected(value: string): boolean {
  return model.value.includes(value);
}

function toggle(value: string): void {
  const opt = props.options.find((o) => o.value === value);
  if (opt?.disabled) return;
  model.value = isSelected(value) ? model.value.filter((v) => v !== value) : [...model.value, value];
}

function remove(value: string): void {
  model.value = model.value.filter((v) => v !== value);
}

const chosen = (): MultiSelectOption[] => props.options.filter((o) => model.value.includes(o.value));
</script>

<template>
  <Popover placement="bottom-start" block>
    <template #trigger="{ open, toggle: togglePopover }">
      <div
        class="flex items-center cursor-pointer select-none"
        :style="{
          gap: '5px',
          width: '100%',
          minHeight: '26px',
          padding: '2px 6px',
          fontSize: '11.5px',
          color: 'var(--c-foreground)',
          background: 'var(--c-input)',
          border: `1px solid ${open ? 'var(--c-primary)' : 'var(--c-border)'}`,
          borderRadius: 'var(--r-lg)',
        }"
        @click="togglePopover"
      >
        <Icon v-if="icon" :name="icon" :size="12" style="color: var(--c-muted); flex: 0 0 auto;" />

        <div class="flex" style="flex: 1; min-width: 0; flex-wrap: wrap; gap: 3px;">
          <span v-if="chosen().length === 0" style="color: var(--c-muted);">{{ placeholder }}</span>
          <span
            v-for="opt in chosen()"
            v-else
            :key="opt.value"
            class="inline-flex items-center"
            style="gap: 4px; height: 17px; padding: 0 3px 0 5px; font-size: 11px; background: var(--c-raised); border: 1px solid var(--c-border); border-radius: 3px;"
          >
            <span
              v-if="opt.dot"
              :style="{ width: '6px', height: '6px', borderRadius: 'var(--r-full)', background: opt.dot }"
            />
            <Icon v-else-if="opt.icon" :name="opt.icon" :size="11" style="color: var(--c-muted); flex: 0 0 auto;" />
            {{ opt.label }}
            <span
              class="inline-flex cursor-pointer"
              style="color: var(--c-muted);"
              @click.stop="remove(opt.value)"
            >
              <Icon name="x" :size="10" />
            </span>
          </span>
        </div>

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
      </div>
    </template>

    <template #default>
      <div role="listbox" style="padding: 3px;">
        <div
          v-for="opt in options"
          :key="opt.value"
          role="option"
          :aria-selected="isSelected(opt.value)"
          :aria-disabled="opt.disabled ? 'true' : undefined"
          class="atl-mi flex items-center"
          :class="{ disabled: opt.disabled }"
          :style="{
            gap: '8px',
            height: '26px',
            padding: '0 7px',
            borderRadius: '3px',
            fontSize: 'var(--fs-sm)',
            cursor: opt.disabled ? 'not-allowed' : 'pointer',
          }"
          @click="toggle(opt.value)"
        >
          <span
            class="inline-flex items-center justify-center shrink-0"
            :style="{
              width: '13px',
              height: '13px',
              borderRadius: '3px',
              border: `1px solid ${isSelected(opt.value) ? 'var(--c-primary)' : 'var(--c-border)'}`,
              background: isSelected(opt.value) ? 'var(--c-primary)' : 'transparent',
              color: 'var(--c-primary-fg)',
            }"
          >
            <Icon v-if="isSelected(opt.value)" name="check" :size="10" :stroke-width="2.8" />
          </span>
          <span
            v-if="opt.dot"
            :style="{ width: '6px', height: '6px', borderRadius: 'var(--r-full)', background: opt.dot, flex: '0 0 auto' }"
          />
          <Icon v-else-if="opt.icon" :name="opt.icon" :size="14" style="color: var(--c-muted); flex: 0 0 auto;" />
          <span style="flex: 1; color: var(--c-foreground);">{{ opt.label }}</span>
          <span
            v-if="opt.disabled"
            style="flex: 0 0 auto; font-size: 9.5px; font-weight: var(--fw-semibold); letter-spacing: 0.06em; text-transform: uppercase; color: var(--c-muted);"
          >
            Soon
          </span>
        </div>
      </div>
    </template>
  </Popover>
</template>
