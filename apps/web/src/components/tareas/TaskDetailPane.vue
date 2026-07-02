<script setup lang="ts">
import { computed, ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import ActivityComments from '@/components/tareas/ActivityComments.vue';
import TaskBody from '@/components/tareas/TaskBody.vue';
import TaskDetailHeader from '@/components/tareas/TaskDetailHeader.vue';
import { useResizablePanel } from '@/composables/useResizablePanel';
import { type TaskViewMode, useUiStore } from '@/stores/ui';

type TaskDto = components['schemas']['TaskDto'];

const props = defineProps<{
  task: TaskDto;
  ws: string;
}>();

const emit = defineEmits<{
  close: [];
  expand: [];
}>();

const ui = useUiStore();

const shareLabel = computed(() => `${props.task.readable_id} · task`);
const isModal = computed(() => ui.effectiveTaskViewMode === 'modal');

// The narrow dock cannot fit the body and the activity+comments panel side by
// side, so a header toggle swaps the whole view between them (ClickUp-style).
const showActivity = ref(false);

// The dock floats over the board as a resizable overlay; its width persists.
const { width: dockWidth, startResize } = useResizablePanel({
  storageKey: 'atlas:task-dock-width',
  min: 340,
  max: 760,
  initial: 440,
});

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
      <div class="atl-tv-modal-body">
        <div class="atl-tv-scroll" style="flex: 1; padding: 20px 32px;">
          <TaskBody :task="task" :ws="ws" layout="wide" :show-secondary="false" />
        </div>
        <aside class="atl-tv-modal-rail">
          <div class="atl-tv-scroll" style="padding: 14px 16px;">
            <ActivityComments :ws="ws" :readable-id="task.readable_id" />
          </div>
        </aside>
      </div>
    </div>
  </div>

  <aside
    v-else
    class="atl-tv-dock"
    :style="{ width: `${dockWidth}px` }"
  >
    <div
      class="atl-tv-dock-resizer"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize panel"
      @mousedown.prevent="startResize"
    />
    <TaskDetailHeader
      :readable-id="task.readable_id"
      :share-label="shareLabel"
      show-expand
      show-activity-toggle
      :activity-open="showActivity"
      @close="emit('close')"
      @expand="emit('expand')"
      @change="onChangeMode"
      @toggle-activity="showActivity = !showActivity"
    />
    <div class="atl-tv-scroll" :style="showActivity ? 'padding: 14px 16px;' : 'padding: 14px 18px;'">
      <ActivityComments v-if="showActivity" :ws="ws" :readable-id="task.readable_id" />
      <TaskBody v-else :task="task" :ws="ws" layout="narrow" :show-secondary="false" />
    </div>
  </aside>
</template>

<style scoped>
.atl-tv-dock {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  z-index: 30;
  display: flex;
  flex-direction: column;
  min-width: 0;
  background: var(--c-background);
  border-left: 1px solid var(--c-border);
  box-shadow: var(--shadow-lg, var(--shadow-md));
}

/* Drag handle straddling the dock's left edge; widens the panel toward the
   board. Kept thin but with a hover cue so it reads as resizable. */
.atl-tv-dock-resizer {
  position: absolute;
  top: 0;
  left: -3px;
  width: 7px;
  height: 100%;
  z-index: 1;
  cursor: col-resize;
  background: transparent;
  transition: background 0.12s;
}

.atl-tv-dock-resizer:hover {
  background: var(--c-primary);
}

.atl-tv-scroll {
  flex: 1;
  overflow: auto;
}

.atl-tv-backdrop {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(5, 8, 12, 0.55);
}

.atl-tv-modal {
  width: min(1280px, 94vw);
  height: min(90vh, 900px);
  display: flex;
  flex-direction: column;
  background: var(--c-background);
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg, var(--r-md));
  box-shadow: var(--shadow-lg, var(--shadow-md));
  overflow: hidden;
}

.atl-tv-modal-body {
  flex: 1;
  display: flex;
  min-height: 0;
}

.atl-tv-modal-rail {
  flex: 0 0 340px;
  display: flex;
  flex-direction: column;
  min-height: 0;
  border-left: 1px solid var(--c-border);
  background: var(--c-panel);
}
</style>
