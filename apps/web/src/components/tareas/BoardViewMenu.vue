<script setup lang="ts">
import { computed, ref } from 'vue';
import { useRouter } from 'vue-router';
import NewViewDialog from '@/components/tareas/NewViewDialog.vue';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import { useBoardsStore } from '@/stores/boards';
import { type TaskViewFiltersDto, useTaskViewsStore } from '@/stores/taskViews';
import { type TaskBoardView, useUiStore } from '@/stores/ui';
import { useUiStateStore } from '@/stores/uiState';
import { useWorkspaceStore } from '@/stores/workspace';

/**
 * Board view switcher (Board · List · Table · Calendar · Timeline). Picking one
 * sets the active layout on the ui store, which the Tasks view renders. The
 * floating surface and its open/dismiss behavior come from the shared Popover.
 */

interface ViewOption {
  id: TaskBoardView;
  label: string;
  icon: string;
}

const DEFAULT_VIEW: ViewOption = { id: 'board', label: 'Board', icon: 'columns-3' };

const VIEWS: ViewOption[] = [
  DEFAULT_VIEW,
  { id: 'list', label: 'List', icon: 'tasks' },
  { id: 'table', label: 'Table', icon: 'dashboard' },
  { id: 'calendar', label: 'Calendar', icon: 'calendar' },
  { id: 'timeline', label: 'Timeline', icon: 'clock' },
];

const router = useRouter();
const ui = useUiStore();
const uiState = useUiStateStore();
const taskViews = useTaskViewsStore();
const workspace = useWorkspaceStore();
const boards = useBoardsStore();

const activeId = computed(() => ui.taskView);
const activeView = computed(() => VIEWS.find((v) => v.id === activeId.value) ?? DEFAULT_VIEW);

function activeLabel(): string {
  return activeView.value.label;
}

function pick(view: ViewOption): void {
  ui.setTaskView(view.id);

  const boardId = boards.board?.id;
  if (boardId !== undefined) uiState.setBoardView(boardId, view.id);
}

const dialogOpen = ref(false);

function newView(): void {
  dialogOpen.value = true;
}

async function onSubmitView(payload: { name: string; filters: TaskViewFiltersDto }): Promise<void> {
  dialogOpen.value = false;
  const ws = workspace.activeWorkspaceSlug;
  if (ws === null) return;

  const created = await taskViews.create(ws, payload);
  if (created !== null) {
    void router.push({ name: 'task-view', params: { viewId: created.id } });
  } else if (taskViews.error) {
    ui.showBanner(taskViews.error, 'error');
  }
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
        <Icon :name="activeView.icon" :size="13" style="color: var(--c-muted); flex: 0 0 auto;" />
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

  <NewViewDialog
    :open="dialogOpen"
    :initial="null"
    @submit="onSubmitView"
    @cancel="dialogOpen = false"
  />
</template>
