<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import { type TaskViewMode, useUiStore } from '@/stores/ui';

const emit = defineEmits<{
  change: [mode: TaskViewMode];
}>();

const ui = useUiStore();

interface ModeOption {
  key: TaskViewMode;
  label: string;
  icon: string;
}

const MODES: ModeOption[] = [
  { key: 'modal', label: 'Dialog', icon: 'app-window' },
  { key: 'full', label: 'Full screen', icon: 'maximize' },
  { key: 'sidebar', label: 'Sidebar', icon: 'panel-right' },
];

const activeIcon = computed(() => MODES.find((m) => m.key === ui.taskViewMode)?.icon ?? 'layout-template');

function pick(mode: TaskViewMode, close: () => void): void {
  ui.setTaskViewMode(mode);
  close();
  emit('change', mode);
}
</script>

<template>
  <Popover placement="bottom-end" width="232px">
    <template #trigger="{ open, toggle }">
      <button
        type="button"
        class="atl-gbtn"
        :class="{ on: open }"
        title="View mode"
        aria-label="Change task view mode"
        style="width: 24px; height: 24px;"
        @click="toggle"
      >
        <Icon :name="activeIcon" :size="15" />
      </button>
    </template>

    <template #default="{ close }">
      <div style="padding: 6px;">
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
            @click="pick(mode.key, close)"
          >
            <span class="atl-tv-thumb" :data-mode="mode.key" :data-active="ui.taskViewMode === mode.key" />
            <span style="font-size: 10.5px; line-height: 1.2; text-align: center;">{{ mode.label }}</span>
          </button>
        </div>
      </div>
    </template>
  </Popover>
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
