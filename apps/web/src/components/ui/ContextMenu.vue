<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue';
import Icon from '@/components/ui/Icon.vue';

export interface MenuItem {
  label: string;
  icon: string;
  action: () => void;
  danger?: boolean;
}

const props = defineProps<{
  open: boolean;
  x: number;
  y: number;
  items: MenuItem[];
}>();

const emit = defineEmits<{
  close: [];
}>();

function clampedX(): string {
  const menuWidth = 180;
  const x = Math.min(props.x, window.innerWidth - menuWidth - 8);
  return `${Math.max(8, x)}px`;
}

function clampedY(): string {
  const menuHeight = props.items.length * 28 + 8;
  const y = Math.min(props.y, window.innerHeight - menuHeight - 8);
  return `${Math.max(8, y)}px`;
}

function onMousedown(event: MouseEvent): void {
  const target = event.target as Node | null;
  const menu = document.getElementById('atl-context-menu');
  if (menu !== null && target !== null && !menu.contains(target)) {
    emit('close');
  }
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    emit('close');
  }
}

function run(item: MenuItem): void {
  item.action();
  emit('close');
}

onMounted(() => {
  window.addEventListener('mousedown', onMousedown);
  window.addEventListener('keydown', onKeydown);
});

onUnmounted(() => {
  window.removeEventListener('mousedown', onMousedown);
  window.removeEventListener('keydown', onKeydown);
});
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      id="atl-context-menu"
      role="menu"
      :style="{
        position: 'fixed',
        top: clampedY(),
        left: clampedX(),
        width: '180px',
        backgroundColor: 'var(--c-panel)',
        border: '1px solid var(--c-border)',
        borderRadius: 'var(--r-md)',
        boxShadow: 'var(--shadow-lg)',
        padding: '4px',
        zIndex: 200,
      }"
    >
      <button
        v-for="item in items"
        :key="item.label"
        type="button"
        role="menuitem"
        class="flex items-center gap-2 w-full text-left atl-row"
        :style="{
          height: '28px',
          padding: '0 9px',
          border: 'none',
          borderRadius: 'var(--r-sm)',
          fontSize: 'var(--fs-sm)',
          fontWeight: 'var(--fw-medium)',
          cursor: 'pointer',
          background: 'transparent',
          color: item.danger === true ? 'var(--c-danger)' : 'var(--c-foreground)',
        }"
        @click="run(item)"
      >
        <Icon :name="item.icon" :size="13" />
        <span class="flex-1">{{ item.label }}</span>
      </button>
    </div>
  </Teleport>
</template>
