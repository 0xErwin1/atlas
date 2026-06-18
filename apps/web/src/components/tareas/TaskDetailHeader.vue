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
  }>(),
  { breadcrumbs: () => [], showExpand: false, showClose: true },
);

const emit = defineEmits<{
  close: [];
  expand: [];
  change: [mode: TaskViewMode];
}>();

const ui = useUiStore();
</script>

<template>
  <header class="atl-tv-header">
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
