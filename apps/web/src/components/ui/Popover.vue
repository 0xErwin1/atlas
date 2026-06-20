<script setup lang="ts">
import { onBeforeUnmount, watch } from 'vue';

/**
 * Single floating-surface primitive: a panel anchored to a trigger, with the
 * open state, placement, click-outside dismissal and Escape handling all in one
 * place. Every anchored menu/dropdown/popover (view switcher, color picker, role
 * menu, account menu, select dropdown) builds on this instead of re-implementing
 * the surface and its behavior. Cursor-positioned menus keep using ContextMenu.
 *
 * Slots:
 *  - `trigger` (scoped: { open, toggle, close }) — the control that opens it.
 *  - default  (scoped: { close }) — the panel content.
 */

type Placement = 'bottom-start' | 'bottom-end' | 'top-start' | 'top-end' | 'right-end';

const props = withDefaults(
  defineProps<{
    placement?: Placement;
    /** Panel width (any CSS length). Omit to size to content. */
    width?: string;
    /** Apply the standard raised menu surface (bg/border/radius/shadow). */
    surface?: boolean;
    /** Make the wrapper fill its container (e.g. full-width select dropdowns). */
    block?: boolean;
  }>(),
  { placement: 'bottom-start', width: '', surface: true, block: false },
);

const open = defineModel<boolean>('open', { default: false });

function close(): void {
  open.value = false;
}

function toggle(): void {
  open.value = !open.value;
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') close();
}

watch(open, (isOpen) => {
  if (isOpen) window.addEventListener('keydown', onKeydown);
  else window.removeEventListener('keydown', onKeydown);
});

onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown));

const PLACEMENT: Record<Placement, Record<string, string>> = {
  'bottom-start': { top: 'calc(100% + 4px)', left: '0' },
  'bottom-end': { top: 'calc(100% + 4px)', right: '0' },
  'top-start': { bottom: 'calc(100% + 4px)', left: '0' },
  'top-end': { bottom: 'calc(100% + 4px)', right: '0' },
  'right-end': { left: 'calc(100% + 8px)', bottom: '0' },
};
</script>

<template>
  <div class="atl-popover" :class="{ block }">
    <slot name="trigger" :open="open" :toggle="toggle" :close="close" />

    <template v-if="open">
      <div
        class="atl-popover-backdrop"
        aria-hidden="true"
        @click="close"
        @contextmenu.prevent="close"
      />
      <div
        class="atl-menu atl-popover-panel"
        :class="{ surface }"
        role="menu"
        :style="{ ...PLACEMENT[placement], width: width || undefined }"
      >
        <slot :close="close" />
      </div>
    </template>
  </div>
</template>

<style scoped>
.atl-popover {
  position: relative;
  display: inline-flex;
}

.atl-popover.block {
  display: flex;
  width: 100%;
}

.atl-popover-backdrop {
  position: fixed;
  inset: 0;
  z-index: 59;
}

.atl-popover-panel {
  position: absolute;
  z-index: 60;
  min-width: 100%;
}

.atl-popover-panel.surface {
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  box-shadow: var(--shadow-md);
}
</style>
