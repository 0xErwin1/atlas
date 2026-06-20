<script setup lang="ts">
import TaskViewModeSwitch from '@/components/tareas/TaskViewModeSwitch.vue';
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
  }>(),
  {
    breadcrumbs: () => [],
    showExpand: false,
    showClose: true,
    showBack: false,
    showNav: false,
    hasPrev: false,
    hasNext: false,
  },
);

const emit = defineEmits<{
  close: [];
  expand: [];
  back: [];
  prev: [];
  next: [];
  change: [mode: TaskViewMode];
}>();

const ui = useUiStore();
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
      type="button"
      class="atl-gbtn"
      style="width: 24px; height: 24px;"
      title="More"
      aria-label="More actions"
      @click="ui.showBanner('That action is coming soon', 'info')"
    >
      <Icon name="more-horizontal" :size="16" />
    </button>

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
