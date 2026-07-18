<script setup lang="ts">
import { nextTick, ref, watch } from 'vue';
import { useOverlayEscape } from '@/composables/useOverlayEscape';

/**
 * Single floating-surface primitive: a panel anchored to a trigger, with the
 * open state, placement, click-outside dismissal and Escape handling all in one
 * place. Every anchored menu/dropdown/popover (view switcher, color picker, role
 * menu, account menu, select dropdown) builds on this instead of re-implementing
 * the surface and its behavior. Cursor-positioned menus keep using ContextMenu.
 *
 * The default surface is `position: absolute`, relative to the trigger wrapper.
 * When `teleport` is set, the panel is rendered into `<body>` as a `position:
 * fixed` surface anchored to the trigger's bounding rect — this is required when
 * an ancestor has `overflow: auto/hidden` and would otherwise clip the panel
 * (e.g. inline editors inside a scrolling task list).
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
    /**
     * Render the panel into `<body>` as a fixed surface anchored to the trigger
     * rect, escaping any clipping `overflow` ancestor. Off by default so every
     * existing consumer keeps the absolute-positioned behavior unchanged.
     */
    teleport?: boolean;
  }>(),
  { placement: 'bottom-start', width: '', surface: true, block: false, teleport: false },
);

const open = defineModel<boolean>('open', { default: false });

const anchor = ref<HTMLElement | null>(null);
const fixedStyle = ref<Record<string, string>>({});

function close(): void {
  open.value = false;
}

function toggle(): void {
  open.value = !open.value;
}

/**
 * Anchors the teleported panel to the trigger rect. Only the start/end edges of
 * the configured placement are honored for the fixed surface (top vs bottom,
 * left vs right); the panel is positioned just outside the trigger on that edge.
 */
function positionFixed(): void {
  const el = anchor.value;
  if (el === null) return;

  const rect = el.getBoundingClientRect();
  const gap = 4;
  const onTop = props.placement.startsWith('top');
  const onEnd = props.placement.endsWith('end');

  const style: Record<string, string> = { position: 'fixed' };

  if (onTop) style.bottom = `${window.innerHeight - rect.top + gap}px`;
  else style.top = `${rect.bottom + gap}px`;

  if (onEnd) style.right = `${window.innerWidth - rect.right}px`;
  else style.left = `${rect.left}px`;

  if (props.width !== '') style.width = props.width;

  fixedStyle.value = style;
}

watch(open, (isOpen) => {
  if (isOpen && props.teleport) void nextTick(positionFixed);
});

useOverlayEscape(open, close);

const PLACEMENT: Record<Placement, Record<string, string>> = {
  'bottom-start': { top: 'calc(100% + 4px)', left: '0' },
  'bottom-end': { top: 'calc(100% + 4px)', right: '0' },
  'top-start': { bottom: 'calc(100% + 4px)', left: '0' },
  'top-end': { bottom: 'calc(100% + 4px)', right: '0' },
  'right-end': { left: 'calc(100% + 8px)', bottom: '0' },
};
</script>

<template>
  <div ref="anchor" class="atl-popover" :class="{ block }">
    <slot name="trigger" :open="open" :toggle="toggle" :close="close" />

    <template v-if="open && !teleport">
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

    <Teleport v-if="teleport" to="body">
      <template v-if="open">
        <div
          class="atl-popover-backdrop fixed"
          aria-hidden="true"
          @click="close"
          @contextmenu.prevent="close"
        />
        <div
          class="atl-menu atl-popover-panel fixed"
          :class="{ surface }"
          role="menu"
          :style="fixedStyle"
        >
          <slot :close="close" />
        </div>
      </template>
    </Teleport>
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

/*
 * The teleported surface renders into <body> as a sibling of hand-rolled dialog
 * overlays (modals/confirm/prompt use the 300-320 band, all teleported to body).
 * It must out-rank that band so a menu opened from inside a dialog is not painted
 * behind it; the backdrop sits just under the panel to keep click-outside working.
 */
.atl-popover-backdrop.fixed {
  z-index: 399;
}

.atl-popover-panel.fixed {
  position: fixed;
  z-index: 400;
  min-width: 0;
}

.atl-popover-panel.surface {
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  box-shadow: var(--shadow-md);
}
</style>
