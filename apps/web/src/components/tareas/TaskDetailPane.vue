<script setup lang="ts">
import { computed } from 'vue';
import type { components } from '@/api/types.d.ts';
import TaskBody from '@/components/tareas/TaskBody.vue';
import TaskDetailHeader from '@/components/tareas/TaskDetailHeader.vue';
import { useBoardsStore } from '@/stores/boards';
import { type TaskViewMode, useUiStore } from '@/stores/ui';

type TaskDto = components['schemas']['TaskDto'];

const props = defineProps<{
  task: TaskDto;
  ws: string;
}>();

const emit = defineEmits<{
  close: [];
  expand: [];
  /** Navigate to a neighbouring board task (prev/next arrows in the dock). */
  navigate: [readableId: string];
}>();

const ui = useUiStore();
const boards = useBoardsStore();

const shareLabel = computed(() => `${props.task.readable_id} · task`);
const isModal = computed(() => ui.effectiveTaskViewMode === 'modal');

// The board's tasks flattened in column order, so the dock's prev/next arrows
// walk the same sequence the user sees on the board.
const orderedIds = computed<string[]>(() =>
  boards.columns.flatMap((c) => boards.tasksByColumn(c.id).map((t) => t.readable_id)),
);

const currentIndex = computed(() => orderedIds.value.indexOf(props.task.readable_id));
const hasPrev = computed(() => currentIndex.value > 0);
const hasNext = computed(() => currentIndex.value >= 0 && currentIndex.value < orderedIds.value.length - 1);

function goPrev(): void {
  const prev = orderedIds.value[currentIndex.value - 1];
  if (hasPrev.value && prev !== undefined) emit('navigate', prev);
}

function goNext(): void {
  const next = orderedIds.value[currentIndex.value + 1];
  if (hasNext.value && next !== undefined) emit('navigate', next);
}

// Picking "Full screen" in the header switch leaves this inline pane (it only
// renders dock/dialog); hand off to the parent to open the standalone route.
function onChangeMode(mode: TaskViewMode): void {
  if (mode === 'full') emit('expand');
}
</script>

<template>
  <div
    v-if="isModal"
    class="atl-tv-backdrop"
    @mousedown.self="emit('close')"
  >
    <div class="atl-tv-modal" @mousedown.stop>
      <TaskDetailHeader
        :readable-id="task.readable_id"
        :share-label="shareLabel"
        show-expand
        @close="emit('close')"
        @expand="emit('expand')"
        @change="onChangeMode"
      />
      <div class="atl-tv-scroll" style="padding: 20px 32px;">
        <TaskBody :task="task" :ws="ws" layout="wide" />
      </div>
    </div>
  </div>

  <aside v-else class="atl-tv-dock">
    <TaskDetailHeader
      :readable-id="task.readable_id"
      :share-label="shareLabel"
      show-expand
      show-nav
      :has-prev="hasPrev"
      :has-next="hasNext"
      @close="emit('close')"
      @expand="emit('expand')"
      @prev="goPrev"
      @next="goNext"
    />
    <div class="atl-tv-scroll" style="padding: 14px 18px;">
      <TaskBody :task="task" :ws="ws" layout="narrow" />
    </div>
  </aside>
</template>

<style scoped>
.atl-tv-dock {
  /* Responsive: shrink with the viewport so opening a task on a narrow window
     doesn't crush the board behind it, but never wider than the design's 460px. */
  width: clamp(320px, 34vw, 460px);
  flex: 0 0 clamp(320px, 34vw, 460px);
  display: flex;
  flex-direction: column;
  min-width: 0;
  background: var(--c-background);
  border-left: 1px solid var(--c-border);
}

.atl-tv-scroll {
  flex: 1;
  overflow: auto;
}

.atl-tv-backdrop {
  position: absolute;
  inset: 0;
  z-index: 40;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(5, 8, 12, 0.55);
}

.atl-tv-modal {
  width: min(820px, 88%);
  height: min(82%, 760px);
  display: flex;
  flex-direction: column;
  background: var(--c-background);
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg, var(--r-md));
  box-shadow: var(--shadow-lg, var(--shadow-md));
  overflow: hidden;
}
</style>
