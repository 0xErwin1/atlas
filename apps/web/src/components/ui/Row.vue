<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';

const props = withDefaults(
  defineProps<{
    label: string;
    icon?: string;
    depth?: number;
    active?: boolean;
    chevron?: boolean;
    open?: boolean;
    muted?: boolean;
    pending?: boolean;
    lock?: boolean;
    right?: string | null;
    disabled?: boolean;
  }>(),
  {
    icon: '',
    depth: 0,
    active: false,
    chevron: false,
    open: false,
    muted: false,
    pending: false,
    lock: false,
    right: null,
    disabled: false,
  },
);

defineEmits<{
  click: [event: MouseEvent];
}>();

const paddingLeft = computed(() => `${8 + props.depth * 14}px`);

const labelColor = computed(() => (props.muted || props.pending ? 'var(--c-muted)' : 'var(--c-foreground)'));

const iconColor = computed(() => (props.active ? 'var(--c-primary)' : 'var(--c-muted)'));
</script>

<template>
  <button
    type="button"
    class="atl-row flex items-center w-full text-left"
    :class="{ on: active }"
    :disabled="disabled"
    :aria-current="active ? 'true' : undefined"
    :style="`
      gap: 6px;
      height: 24px;
      padding-left: ${paddingLeft};
      padding-right: 8px;
      border: none;
      cursor: ${disabled ? 'default' : 'pointer'};
      background: ${active ? 'var(--c-selection)' : 'transparent'};
      box-shadow: ${active ? 'inset 2px 0 0 var(--c-primary)' : 'none'};
      color: ${labelColor};
      font-size: var(--fs-sm);
      font-weight: ${active ? 'var(--fw-semibold)' : 'var(--fw-medium)'};
      user-select: none;
    `"
    @click="$emit('click', $event)"
  >
    <span
      v-if="chevron"
      class="flex items-center justify-center shrink-0"
      style="width: 12px; color: var(--c-muted);"
    >
      <Icon :name="open ? 'chevron-down' : 'chevron-right'" :size="12" />
    </span>
    <span v-else style="width: 12px; flex-shrink: 0;" />

    <Icon
      v-if="icon"
      :name="icon"
      :size="13"
      :style="`color: ${iconColor}; flex-shrink: 0;`"
    />

    <span
      class="flex-1 truncate"
      :style="`text-decoration: ${pending ? 'line-through' : 'none'};`"
    >
      {{ label }}
    </span>

    <span
      v-if="pending"
      title="Broken link · pending note"
      style="width: 6px; height: 6px; border-radius: 9999px; background: var(--c-danger); flex-shrink: 0;"
      aria-hidden="true"
    />

    <Icon
      v-if="lock"
      name="lock"
      :size="11"
      style="color: var(--c-muted); flex-shrink: 0;"
    />

    <span
      v-if="right != null"
      class="shrink-0"
      style="font-family: var(--font-mono); font-size: 10px; color: var(--c-muted);"
    >
      {{ right }}
    </span>
  </button>
</template>
