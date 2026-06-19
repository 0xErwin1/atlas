<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import { type TaskViewMode, useUiStore } from '@/stores/ui';

const emit = defineEmits<{
  change: [mode: TaskViewMode];
}>();

const ui = useUiStore();

interface ModeOption {
  key: TaskViewMode;
  label: string;
}

const MODES: ModeOption[] = [
  { key: 'modal', label: 'Dialog' },
  { key: 'full', label: 'Full screen' },
  { key: 'sidebar', label: 'Sidebar' },
];

const open = ref(false);
const root = ref<HTMLElement | null>(null);

function onDocMousedown(event: MouseEvent): void {
  if (root.value && !root.value.contains(event.target as Node)) open.value = false;
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') open.value = false;
}

watch(open, (isOpen) => {
  if (isOpen) {
    window.addEventListener('mousedown', onDocMousedown);
    window.addEventListener('keydown', onKeydown);
  } else {
    window.removeEventListener('mousedown', onDocMousedown);
    window.removeEventListener('keydown', onKeydown);
  }
});

onBeforeUnmount(() => {
  window.removeEventListener('mousedown', onDocMousedown);
  window.removeEventListener('keydown', onKeydown);
});

function pick(mode: TaskViewMode): void {
  ui.setTaskViewMode(mode);
  open.value = false;
  emit('change', mode);
}
</script>

<template>
  <div ref="root" style="position: relative;">
    <button
      type="button"
      class="atl-gbtn"
      :class="{ on: open }"
      title="View mode"
      aria-label="Change task view mode"
      style="width: 26px; height: 26px;"
      @click="open = !open"
    >
      <Icon name="layout-template" :size="15" />
    </button>

    <div
      v-if="open"
      class="atl-menu"
      role="menu"
      style="
        position: absolute;
        top: 32px;
        right: 0;
        z-index: 60;
        width: 236px;
        padding: 6px;
        background: var(--c-raised);
        border: 1px solid var(--c-border);
        border-radius: var(--r-md);
        box-shadow: var(--shadow-lg, var(--shadow-md));
      "
    >
      <div
        style="
          padding: 4px 6px 6px;
          font-size: var(--fs-xs);
          font-weight: var(--fw-semibold);
          letter-spacing: 0.06em;
          text-transform: uppercase;
          color: var(--c-muted);
        "
      >
        Open task as
      </div>

      <div class="flex" style="gap: 6px;">
        <button
          v-for="mode in MODES"
          :key="mode.key"
          type="button"
          role="menuitemradio"
          :aria-checked="ui.taskViewMode === mode.key"
          class="atl-tv-modeopt flex flex-col items-center"
          :class="{ active: ui.taskViewMode === mode.key }"
          style="flex: 1; gap: 6px; padding: 8px 4px; border-radius: var(--r-sm); cursor: pointer;"
          @click="pick(mode.key)"
        >
          <span class="atl-tv-thumb" :data-mode="mode.key" :data-active="ui.taskViewMode === mode.key" />
          <span style="font-size: 10.5px; line-height: 1.2; text-align: center;">{{ mode.label }}</span>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.atl-tv-modeopt {
  border: 1px solid transparent;
  background: transparent;
  color: var(--c-muted);
  font-weight: var(--fw-medium);
}

.atl-tv-modeopt:hover {
  background: var(--c-raised-hover, var(--c-selection));
}

.atl-tv-modeopt.active {
  border-color: var(--c-primary);
  background: var(--c-selection);
  color: var(--c-primary);
  font-weight: var(--fw-semibold);
}

/* Tiny layout diagram per mode, mirroring the hi-fi mode picker. */
.atl-tv-thumb {
  position: relative;
  width: 56px;
  height: 38px;
  border-radius: 3px;
  border: 1px solid var(--c-border);
  background: var(--c-background);
  overflow: hidden;
}

.atl-tv-thumb[data-active='true'] {
  border-color: var(--c-primary);
}

/* the always-present mini app rail */
.atl-tv-thumb::before {
  content: '';
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 7px;
  background: rgba(179, 177, 173, 0.18);
}

.atl-tv-thumb::after {
  content: '';
  position: absolute;
  background: var(--c-muted);
  opacity: 0.5;
  border-radius: 2px;
}

.atl-tv-thumb[data-active='true']::after {
  background: var(--c-primary);
  opacity: 0.9;
}

.atl-tv-thumb[data-mode='modal']::after {
  left: 16px;
  top: 8px;
  right: 8px;
  bottom: 8px;
}

.atl-tv-thumb[data-mode='full']::after {
  left: 9px;
  top: 4px;
  right: 3px;
  bottom: 4px;
}

.atl-tv-thumb[data-mode='sidebar']::after {
  right: 0;
  top: 0;
  bottom: 0;
  width: 22px;
  border-radius: 0;
}
</style>
