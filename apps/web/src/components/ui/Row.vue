<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';

const props = withDefaults(
  defineProps<{
    label: string;
    icon?: string;
    depth?: number;
    depthStep?: number;
    active?: boolean;
    chevron?: boolean;
    open?: boolean;
    muted?: boolean;
    pending?: boolean;
    lock?: boolean;
    right?: string | null;
    disabled?: boolean;
    /** Overrides the icon color; when unset the icon follows the active/muted default. */
    iconColor?: string;
    /** Show a kebab (⋯) button on hover that emits `menu` to open a context menu. */
    menu?: boolean;
    menuIcon?: string;
    menuLabel?: string;
    menuAlwaysVisible?: boolean;
  }>(),
  {
    icon: '',
    depth: 0,
    depthStep: 14,
    active: false,
    chevron: false,
    open: false,
    muted: false,
    pending: false,
    lock: false,
    right: null,
    disabled: false,
    iconColor: undefined,
    menu: false,
    menuIcon: 'more-horizontal',
    menuLabel: 'More actions',
    menuAlwaysVisible: false,
  },
);

defineEmits<{
  click: [event: MouseEvent];
  menu: [event: MouseEvent];
}>();

const paddingLeft = computed(() => `${8 + props.depth * props.depthStep}px`);

const labelColor = computed(() => (props.muted || props.pending ? 'var(--c-muted)' : 'var(--c-foreground)'));

const iconColor = computed(() => props.iconColor ?? (props.active ? 'var(--c-primary)' : 'var(--c-muted)'));
</script>

<template>
  <div class="atl-row-wrap">
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

    <button
      v-if="menu && !disabled"
      type="button"
      class="atl-row-kebab"
      :class="{ 'always-visible': menuAlwaysVisible }"
      :title="menuLabel"
      :aria-label="menuLabel"
      @click.stop="$emit('menu', $event)"
    >
      <Icon :name="menuIcon" :size="14" />
    </button>
  </div>
</template>

<style scoped>
.atl-row-wrap {
  position: relative;
}

.atl-row-kebab {
  position: absolute;
  right: 6px;
  top: 50%;
  transform: translateY(-50%);
  display: flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  padding: 0;
  border: none;
  background: var(--c-panel);
  color: var(--c-muted);
  border-radius: var(--r-sm);
  cursor: pointer;
  opacity: 0;
}

.atl-row-wrap:hover .atl-row-kebab {
  opacity: 1;
}

.atl-row-wrap:focus-within .atl-row-kebab {
  opacity: 1;
}

.atl-row-kebab.always-visible {
  opacity: 1;
}

.atl-row-kebab:hover {
  background: var(--c-raised);
  color: var(--c-foreground);
}
</style>
