<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from 'vue';
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
 * (e.g. inline editors inside a scrolling task list). The teleported surface
 * tracks the trigger while scrolling/resizing, moves focus into the panel on
 * open, and restores it to the trigger on close.
 *
 * Slots:
 *  - `trigger` (scoped: { open, toggle, close }) — the control that opens it.
 *  - default  (scoped: { close }) — the panel content.
 */

type Placement = 'bottom-start' | 'bottom-end' | 'top-start' | 'top-end' | 'right-end';

type PanelRole = 'menu' | 'dialog' | 'listbox';

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
    /**
     * ARIA role of the panel itself, so consumers never nest a second role
     * inside it: a date picker passes `dialog`, a select passes `listbox`.
     */
    role?: PanelRole;
    /** Accessible name for the panel (e.g. required when `role` is `dialog`). */
    ariaLabel?: string;
  }>(),
  {
    placement: 'bottom-start',
    width: '',
    surface: true,
    block: false,
    teleport: false,
    role: 'menu',
    ariaLabel: undefined,
  },
);

const open = defineModel<boolean>('open', { default: false });

const anchor = ref<HTMLElement | null>(null);
const panel = ref<HTMLElement | null>(null);
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
 * left vs right). When the panel's measured size is available, a bottom
 * placement that would overflow the viewport flips to the top (and vice versa,
 * when the preferred top side doesn't fit) and the panel is clamped to the
 * horizontal viewport edges; without a measurement it keeps the plain
 * anchored-edge position.
 */
function positionFixed(): void {
  const el = anchor.value;
  if (el === null) return;

  const rect = el.getBoundingClientRect();
  const gap = 4;
  const panelWidth = panel.value?.offsetWidth ?? 0;
  const panelHeight = panel.value?.offsetHeight ?? 0;

  let onTop = props.placement.startsWith('top');
  if (panelHeight > 0) {
    const overflowsBelow = rect.bottom + gap + panelHeight > window.innerHeight;
    const fitsAbove = rect.top - gap - panelHeight >= 0;
    if (!onTop && overflowsBelow && fitsAbove) onTop = true;
    else if (onTop && !fitsAbove && !overflowsBelow) onTop = false;
  }

  const style: Record<string, string> = { position: 'fixed' };

  if (onTop) style.bottom = `${window.innerHeight - rect.top + gap}px`;
  else style.top = `${rect.bottom + gap}px`;

  const onEnd = props.placement.endsWith('end');
  if (panelWidth > 0) {
    const preferredLeft = onEnd ? rect.right - panelWidth : rect.left;
    const maxLeft = Math.max(window.innerWidth - panelWidth, 0);
    style.left = `${Math.min(Math.max(preferredLeft, 0), maxLeft)}px`;
  } else if (onEnd) {
    style.right = `${window.innerWidth - rect.right}px`;
  } else {
    style.left = `${rect.left}px`;
  }

  if (props.width !== '') style.width = props.width;

  fixedStyle.value = style;
}

function onViewportChange(): void {
  positionFixed();
}

function attachViewportListeners(): void {
  window.addEventListener('scroll', onViewportChange, { capture: true, passive: true });
  window.addEventListener('resize', onViewportChange, { passive: true });
}

function detachViewportListeners(): void {
  window.removeEventListener('scroll', onViewportChange, { capture: true });
  window.removeEventListener('resize', onViewportChange);
}

let restoreFocusTo: HTMLElement | null = null;

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Moves focus into the teleported panel so Tab starts from the panel instead
 * of walking the page under it; falls back to the panel itself (which carries
 * `tabindex="-1"`) when it has no focusable content.
 */
function focusPanel(): void {
  const el = panel.value;
  if (el === null) return;

  const target = el.querySelector<HTMLElement>(FOCUSABLE) ?? el;
  target.focus();
}

watch(open, (isOpen) => {
  if (!props.teleport) return;

  if (isOpen) {
    restoreFocusTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    attachViewportListeners();
    void nextTick(() => {
      positionFixed();
      focusPanel();
    });
    return;
  }

  detachViewportListeners();
  restoreFocusTo?.focus();
  restoreFocusTo = null;
});

onBeforeUnmount(detachViewportListeners);

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
        :role="role"
        :aria-label="ariaLabel"
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
          ref="panel"
          class="atl-menu atl-popover-panel fixed"
          :class="{ surface }"
          :role="role"
          :aria-label="ariaLabel"
          tabindex="-1"
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

.atl-popover-panel.fixed:focus {
  outline: none;
}

.atl-popover-panel.surface {
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  box-shadow: var(--shadow-md);
}
</style>
