<script setup lang="ts">
import { computed, ref } from 'vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import { useContextMenu } from '@/composables/useContextMenu';

export interface Tab {
  id: string;
  name: string;
  icon: string;
  active?: boolean;
  dirty?: boolean;
}

const props = withDefaults(
  defineProps<{
    tabs?: Tab[];
    closable?: boolean;
  }>(),
  {
    tabs: () => [],
    closable: false,
  },
);

const emit = defineEmits<{
  select: [id: string];
  close: [id: string];
  'close-others': [id: string];
  'close-right': [id: string];
  'close-all': [];
}>();

const { open: menuOpen, x: menuX, y: menuY, openAt, close: closeMenu } = useContextMenu();
const menuTabId = ref<string | null>(null);

function openTabMenu(event: MouseEvent, id: string): void {
  menuTabId.value = id;
  openAt(event);
}

const menuItems = computed<MenuItem[]>(() => {
  const id = menuTabId.value;
  if (id === null) return [];
  const idx = props.tabs.findIndex((t) => t.id === id);
  const onlyOne = props.tabs.length <= 1;
  const isLast = idx === props.tabs.length - 1;
  return [
    { label: 'Close', icon: 'x', action: () => emit('close', id) },
    {
      label: 'Close others',
      icon: 'x',
      disabled: onlyOne,
      action: () => emit('close-others', id),
    },
    {
      label: 'Close to the right',
      icon: 'arrow-right',
      disabled: isLast,
      action: () => emit('close-right', id),
    },
    { sep: true },
    { label: 'Close all', icon: 'x', action: () => emit('close-all') },
  ];
});
</script>

<template>
  <div
    class="flex items-end"
    style="
      height: 32px;
      flex: 0 0 32px;
      background-color: var(--c-panel);
      border-bottom: 1px solid var(--c-border);
      padding: 0 4px;
      gap: 2px;
    "
  >
    <div
      v-for="tab in tabs"
      :key="tab.id"
      class="atl-tab inline-flex items-center"
      :title="tab.name"
      role="tab"
      :aria-selected="tab.active === true"
      :style="`
        gap: 8px;
        height: 28px;
        padding: 0 8px 0 10px;
        border-radius: 2px 2px 0 0;
        background: ${tab.active ? 'var(--c-background)' : 'transparent'};
        color: ${tab.active ? 'var(--c-foreground)' : 'var(--c-muted)'};
        font-weight: ${tab.active ? 'var(--fw-semibold)' : 'var(--fw-medium)'};
        font-size: var(--fs-sm);
        cursor: pointer;
        box-shadow: ${tab.active ? 'inset 0 -2px 0 var(--c-primary)' : 'none'};
        max-width: 200px;
        min-width: 110px;
      `"
      @click="$emit('select', tab.id)"
      @contextmenu.prevent.stop="openTabMenu($event, tab.id)"
    >
      <Icon :name="tab.icon" :size="12" style="flex-shrink: 0;" />
      <span class="flex-1 truncate">{{ tab.name }}</span>
      <span
        v-if="tab.dirty"
        style="width: 6px; height: 6px; border-radius: 9999px; background: var(--c-primary); flex-shrink: 0;"
        aria-hidden="true"
      />
      <span
        v-if="closable"
        class="atl-x inline-flex items-center justify-center"
        style="width: 14px; height: 14px; border-radius: 2px; color: var(--c-muted); font-size: 13px; flex-shrink: 0;"
        role="button"
        :aria-label="`Close ${tab.name}`"
        @click.stop="$emit('close', tab.id)"
      >
        ×
      </span>
    </div>

    <div class="flex items-center" style="margin-left: auto; gap: 4px; padding-right: 4px; align-self: center;">
      <slot name="right" />
    </div>

    <ContextMenu :open="menuOpen" :x="menuX" :y="menuY" :items="menuItems" @close="closeMenu" />
  </div>
</template>
