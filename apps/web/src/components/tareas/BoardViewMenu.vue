<script setup lang="ts">
import { ref } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import { useUiStore } from '@/stores/ui';

/**
 * Board view switcher (Board · List · Calendar · Table · Timeline). Only the
 * Board view is implemented; the others are planned layouts, so selecting one
 * surfaces a "coming soon" banner instead of silently doing nothing. The trigger
 * and menu mirror the hi-fi `ViewMenu`.
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
const open = ref(false);
const activeId = ref('board');

function toggle(): void {
  open.value = !open.value;
}

function close(): void {
  open.value = false;
}

function pick(view: ViewOption): void {
  open.value = false;
  if (view.ready) {
    activeId.value = view.id;
    return;
  }
  ui.showBanner(`${view.label} view is coming soon`, 'info');
}

function newView(): void {
  open.value = false;
  ui.showBanner('Custom views are coming soon', 'info');
}
</script>

<template>
  <div style="position: relative;">
    <button
      type="button"
      class="atl-dd"
      :title="`View: ${VIEWS.find((v) => v.id === activeId)?.label ?? 'Board'}`"
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
      <span style="white-space: nowrap;">{{ VIEWS.find((v) => v.id === activeId)?.label ?? 'Board' }}</span>
      <Icon name="chevron-down" :size="12" style="color: var(--c-muted); flex: 0 0 auto;" />
    </button>

    <template v-if="open">
      <div
        aria-hidden="true"
        style="position: fixed; inset: 0; z-index: 59;"
        @click="close"
        @contextmenu.prevent="close"
      />
      <div
        class="atl-menu"
        role="menu"
        style="
          position: absolute;
          top: 32px;
          left: 0;
          z-index: 60;
          width: 210px;
          background: var(--c-raised);
          border: 1px solid var(--c-border);
          border-radius: var(--r-md);
          box-shadow: var(--shadow-lg);
          padding: 5px 0;
        "
      >
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
          @click="pick(view)"
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

        <div class="atl-vmi" role="menuitem" @click="newView">
          <Icon name="plus" :size="14" style="color: var(--c-muted); flex: 0 0 auto;" />
          <span style="flex: 1;">New view</span>
        </div>
      </div>
    </template>
  </div>
</template>
