<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue';
import WorkspaceSwitcher from '@/components/shell/WorkspaceSwitcher.vue';

const SIDEBAR_MIN = 218;
const SIDEBAR_DEFAULT = 218;
const SIDEBAR_MAX = 480;
const SIDEBAR_STEP = 16;

const sidebarWidth = ref(SIDEBAR_DEFAULT);

let activePointerId: number | null = null;
let startX = 0;
let startWidth = SIDEBAR_DEFAULT;
let previousCursor = '';
let previousUserSelect = '';

function clampWidth(width: number): number {
  return Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, width));
}

function finishResize(): void {
  if (activePointerId === null) return;

  activePointerId = null;
  window.removeEventListener('pointermove', onPointerMove);
  window.removeEventListener('pointerup', finishResize);
  window.removeEventListener('pointercancel', finishResize);
  window.removeEventListener('blur', finishResize);
  document.body.style.cursor = previousCursor;
  document.body.style.userSelect = previousUserSelect;
}

function onPointerMove(event: PointerEvent): void {
  if (event.pointerId !== activePointerId) return;

  sidebarWidth.value = clampWidth(startWidth + event.clientX - startX);
}

function onPointerDown(event: PointerEvent): void {
  if (event.button !== 0 || activePointerId !== null) return;

  activePointerId = event.pointerId;
  startX = event.clientX;
  startWidth = sidebarWidth.value;
  previousCursor = document.body.style.cursor;
  previousUserSelect = document.body.style.userSelect;
  document.body.style.cursor = 'col-resize';
  document.body.style.userSelect = 'none';
  window.addEventListener('pointermove', onPointerMove);
  window.addEventListener('pointerup', finishResize);
  window.addEventListener('pointercancel', finishResize);
  window.addEventListener('blur', finishResize);
}

function onSeparatorKeydown(event: KeyboardEvent): void {
  const nextWidth = {
    ArrowLeft: sidebarWidth.value - SIDEBAR_STEP,
    ArrowDown: sidebarWidth.value - SIDEBAR_STEP,
    ArrowRight: sidebarWidth.value + SIDEBAR_STEP,
    ArrowUp: sidebarWidth.value + SIDEBAR_STEP,
    Home: SIDEBAR_MIN,
    End: SIDEBAR_MAX,
  }[event.key];

  if (nextWidth === undefined) return;

  event.preventDefault();
  sidebarWidth.value = clampWidth(nextWidth);
}

onBeforeUnmount(finishResize);
</script>

<template>
  <aside
    :style="{
      width: `${sidebarWidth}px`,
      flexGrow: 0,
      flexBasis: `${sidebarWidth}px`,
      minWidth: `${SIDEBAR_MIN}px`,
      maxWidth: `${SIDEBAR_MAX}px`,
      flexShrink: 0,
      backgroundColor: 'var(--c-panel)',
      borderRight: '1px solid var(--c-border)',
      height: '100%',
      display: 'flex',
      flexDirection: 'column',
      overflow: 'hidden',
      position: 'relative',
    }"
  >
    <div
      class="flex items-center"
      style="
        height: var(--h-header);
        padding: 0 6px 0 6px;
        gap: 8px;
        border-bottom: 1px solid var(--c-border);
        flex-shrink: 0;
      "
    >
      <div class="flex-1 min-w-0">
        <WorkspaceSwitcher />
      </div>

      <div class="flex items-center" style="gap: 2px;">
        <slot name="header-actions" />
      </div>
    </div>

    <div class="flex-1 min-h-0 overflow-hidden">
      <slot />
    </div>

    <div
      v-if="$slots.footer"
      style="border-top: 1px solid var(--c-border); padding: 8px; flex-shrink: 0;"
    >
      <slot name="footer" />
    </div>

    <div
      class="atl-sidebar-resize-handle"
      role="separator"
      aria-label="Resize sidebar"
      aria-orientation="vertical"
      :aria-valuemin="SIDEBAR_MIN"
      :aria-valuemax="SIDEBAR_MAX"
      :aria-valuenow="sidebarWidth"
      tabindex="0"
      @pointerdown="onPointerDown"
      @keydown="onSeparatorKeydown"
    />
  </aside>
</template>

<style scoped>
.atl-sidebar-resize-handle {
  position: absolute;
  top: 0;
  right: -4px;
  bottom: 0;
  z-index: 1;
  width: 8px;
  cursor: col-resize;
  touch-action: none;
}

.atl-sidebar-resize-handle:focus-visible {
  outline: 2px solid var(--c-primary);
  outline-offset: -2px;
}
</style>
