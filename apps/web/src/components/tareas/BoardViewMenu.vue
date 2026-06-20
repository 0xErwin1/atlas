<script setup lang="ts">
import { ref } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import { useUiStore } from '@/stores/ui';

/**
 * Board view switcher (Board · List · Calendar · Table · Timeline). Only the
 * Board view is implemented; the others are planned layouts, so selecting one
 * surfaces a "coming soon" banner. The floating surface and its open/dismiss
 * behavior come from the shared Popover.
 */

interface ViewOption {
  id: string;
  label: string;
  icon: string;
  ready: boolean;
}

const VIEWS: ViewOption[] = [
  { id: 'board', label: 'Board', icon: 'columns-3', ready: true },
  { id: 'list', label: 'List', icon: 'tasks', ready: false },
  { id: 'calendar', label: 'Calendar', icon: 'calendar', ready: false },
  { id: 'table', label: 'Table', icon: 'dashboard', ready: false },
  { id: 'timeline', label: 'Timeline', icon: 'clock', ready: false },
];

const ui = useUiStore();
const activeId = ref('board');

function activeLabel(): string {
  return VIEWS.find((v) => v.id === activeId.value)?.label ?? 'Board';
}

function pick(view: ViewOption): void {
  if (view.ready) {
    activeId.value = view.id;
    return;
  }
  ui.showBanner(`${view.label} view is coming soon`, 'info');
}

function newView(): void {
  ui.showBanner('Custom views are coming soon', 'info');
}
</script>

<template>
  <Popover placement="bottom-start" width="210px">
    <template #trigger="{ open, toggle }">
      <button
        type="button"
        class="atl-dd"
        :title="`View: ${activeLabel()}`"
        aria-haspopup="menu"
        :aria-expanded="open"
        style="
          display: inline-flex;
          align-items: center;
          gap: 7px;
          height: 28px;
          padding: 0 9px;
          font-size: var(--fs-sm);
          color: var(--c-foreground);
          background: var(--c-secondary);
          border: 1px solid var(--c-border);
          border-radius: var(--r-sm);
          cursor: pointer;
        "
        :style="{ borderColor: open ? 'var(--c-primary)' : 'var(--c-border)' }"
        @click="toggle"
      >
        <Icon name="columns-3" :size="13" style="color: var(--c-muted); flex: 0 0 auto;" />
        <span style="white-space: nowrap;">{{ activeLabel() }}</span>
        <Icon name="chevron-down" :size="12" style="color: var(--c-muted); flex: 0 0 auto;" />
      </button>
    </template>

    <template #default="{ close }">
      <div style="padding: 5px 0;">
        <div
          style="
            font-size: var(--fs-xs);
            font-weight: var(--fw-semibold);
            letter-spacing: 0.06em;
            text-transform: uppercase;
            color: var(--c-muted);
            padding: 4px 12px 5px;
          "
        >
          View as
        </div>

        <div
          v-for="view in VIEWS"
          :key="view.id"
          class="atl-vmi"
          :class="{ on: view.id === activeId }"
          role="menuitem"
          @click="pick(view), close()"
        >
          <Icon
            :name="view.icon"
            :size="14"
            :style="{ color: view.id === activeId ? 'var(--c-primary)' : 'var(--c-muted)', flex: '0 0 auto' }"
          />
          <span style="flex: 1;">{{ view.label }}</span>
          <Icon
            v-if="view.id === activeId"
            name="check"
            :size="13"
            style="color: var(--c-primary); flex: 0 0 auto;"
          />
        </div>

        <div aria-hidden="true" style="height: 1px; background: var(--c-border); margin: 5px 0;" />

        <div class="atl-vmi" role="menuitem" @click="newView(), close()">
          <Icon name="plus" :size="14" style="color: var(--c-muted); flex: 0 0 auto;" />
          <span style="flex: 1;">New view</span>
        </div>
      </div>
    </template>
  </Popover>
</template>
