<script setup lang="ts">
import { computed, ref } from 'vue';
import TaskViewModeSwitch from '@/components/tareas/TaskViewModeSwitch.vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import { type TaskViewMode, useUiStore } from '@/stores/ui';

const props = withDefaults(
  defineProps<{
    readableId: string;
    shareLabel: string;
    breadcrumbs?: string[];
    showExpand?: boolean;
    showClose?: boolean;
    showBack?: boolean;
    /** Prev/next task arrows, shown in the sidebar-dock variant. */
    showNav?: boolean;
    hasPrev?: boolean;
    hasNext?: boolean;
    /** Right inspector collapse toggle, shown only in the full-screen view. */
    showInspectorToggle?: boolean;
    inspectorOpen?: boolean;
    /** Activity+comments panel toggle, shown in the narrow sidebar dock where the
     * panel replaces the body instead of sitting beside it. */
    showActivityToggle?: boolean;
    activityOpen?: boolean;
  }>(),
  {
    breadcrumbs: () => [],
    showExpand: false,
    showClose: true,
    showBack: false,
    showNav: false,
    hasPrev: false,
    hasNext: false,
    showInspectorToggle: false,
    inspectorOpen: true,
    showActivityToggle: false,
    activityOpen: false,
  },
);

const emit = defineEmits<{
  close: [];
  expand: [];
  back: [];
  prev: [];
  next: [];
  change: [mode: TaskViewMode];
  toggleInspector: [];
  toggleActivity: [];
}>();

const ui = useUiStore();

const menuOpen = ref(false);
const menuX = ref(0);
const menuY = ref(0);

const MENU_WIDTH = 210;

function openMenu(event: MouseEvent): void {
  const btn = event.currentTarget as HTMLElement | null;
  if (btn !== null) {
    const rect = btn.getBoundingClientRect();
    menuX.value = rect.right - MENU_WIDTH;
    menuY.value = rect.bottom + 4;
  }
  menuOpen.value = true;
}

function taskUrl(): string {
  return `${window.location.origin}/t/task/${props.readableId}`;
}

async function copy(text: string, label: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    ui.showBanner(`${label} copied`, 'success');
  } catch {
    ui.showBanner('Clipboard is not available', 'error');
  }
}

const menuItems = computed<MenuItem[]>(() => [
  { label: 'Copy link', icon: 'link', action: () => copy(taskUrl(), 'Link') },
  { label: 'Copy ID', icon: 'hash', action: () => copy(props.readableId, 'ID') },
  { label: 'Open in new tab', icon: 'external-link', action: () => window.open(taskUrl(), '_blank') },
]);
</script>

<template>
  <header class="atl-tv-header">
    <button
      v-if="showBack"
      type="button"
      class="atl-gbtn"
      style="width: 26px; height: 26px;"
      title="Back to board"
      aria-label="Back to board"
      @click="emit('back')"
    >
      <Icon name="arrow-left" :size="16" />
    </button>

    <div v-if="showNav" class="flex" style="gap: 2px;">
      <button
        type="button"
        class="atl-gbtn"
        style="width: 22px; height: 22px;"
        title="Previous task"
        aria-label="Previous task"
        :disabled="!hasPrev"
        @click="emit('prev')"
      >
        <Icon name="chevron-down" :size="14" style="transform: rotate(90deg);" />
      </button>
      <button
        type="button"
        class="atl-gbtn"
        style="width: 22px; height: 22px;"
        title="Next task"
        aria-label="Next task"
        :disabled="!hasNext"
        @click="emit('next')"
      >
        <Icon name="chevron-down" :size="14" style="transform: rotate(-90deg);" />
      </button>
    </div>

    <nav v-if="breadcrumbs.length" class="atl-tv-crumb" aria-label="Breadcrumb">
      <template v-for="(part, i) in breadcrumbs" :key="i">
        <span v-if="i > 0" class="atl-tv-crumb-sep">/</span>
        <span class="atl-tv-crumb-part" :class="{ last: i === breadcrumbs.length - 1 }">{{ part }}</span>
      </template>
    </nav>
    <span v-else class="atl-tv-id">{{ readableId }}</span>

    <span style="flex: 1;" />

    <button type="button" class="atl-gbtn" style="height: 26px;" title="Share" @click="ui.openShare(shareLabel)">
      <Icon name="users" :size="14" />
      Share
    </button>

    <button
      v-if="showActivityToggle"
      type="button"
      class="atl-gbtn"
      :class="{ on: activityOpen }"
      style="width: 26px; height: 26px;"
      :title="activityOpen ? 'Back to task' : 'Activity & comments'"
      :aria-label="activityOpen ? 'Back to task' : 'Activity and comments'"
      @click="emit('toggleActivity')"
    >
      <Icon :name="activityOpen ? 'file-text' : 'message-square'" :size="15" />
    </button>

    <TaskViewModeSwitch @change="(m) => emit('change', m)" />

    <button
      v-if="showExpand"
      type="button"
      class="atl-gbtn"
      style="width: 26px; height: 26px;"
      title="Expand to full screen"
      @click="emit('expand')"
    >
      <Icon name="maximize-2" :size="15" />
    </button>

    <button
      v-if="showInspectorToggle"
      type="button"
      class="atl-gbtn"
      style="width: 26px; height: 26px;"
      :title="inspectorOpen ? 'Hide details panel' : 'Show details panel'"
      :aria-label="inspectorOpen ? 'Hide details panel' : 'Show details panel'"
      @click="emit('toggleInspector')"
    >
      <Icon :name="inspectorOpen ? 'panel-right-close' : 'panel-right-open'" :size="16" />
    </button>

    <button
      type="button"
      class="atl-gbtn"
      style="width: 24px; height: 24px;"
      title="More"
      aria-label="More actions"
      @click="openMenu"
    >
      <Icon name="more-horizontal" :size="16" />
    </button>

    <ContextMenu
      :open="menuOpen"
      :x="menuX"
      :y="menuY"
      :items="menuItems"
      :width="MENU_WIDTH"
      @close="menuOpen = false"
    />

    <button
      v-if="showClose"
      type="button"
      class="atl-gbtn"
      style="width: 26px; height: 26px;"
      title="Close"
      aria-label="Close task"
      @click="emit('close')"
    >
      <Icon name="x" :size="16" />
    </button>
  </header>
</template>

<style scoped>
.atl-tv-header {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 44px;
  flex: 0 0 44px;
  padding: 0 8px 0 12px;
  border-bottom: 1px solid var(--c-border);
  background: var(--c-panel);
}

.atl-tv-id {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-tv-crumb {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  overflow: hidden;
}

.atl-tv-crumb-part {
  font-size: var(--fs-sm);
  color: var(--c-muted);
  white-space: nowrap;
}

.atl-tv-crumb-part.last {
  color: var(--c-foreground);
  font-weight: var(--fw-medium);
}

.atl-tv-crumb-sep {
  color: var(--c-muted);
  opacity: 0.6;
}
</style>
