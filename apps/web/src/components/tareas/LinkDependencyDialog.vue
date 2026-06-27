<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue';
import type { components } from '@/api/types.d.ts';
import ReferenceAdd from '@/components/tareas/ReferenceAdd.vue';
import Icon from '@/components/ui/Icon.vue';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useUiStore } from '@/stores/ui';

/**
 * Modal entry point for the task body's "Link or add dependency" action. Wraps
 * the shared ReferenceAdd preselected to a `blocks` dependency, so the flow works
 * the same in every task layout (dock, dialog, full screen) rather than depending
 * on whichever surface happens to host the inline References section.
 */
const props = defineProps<{
  ws: string;
  readableId: string;
}>();

const emit = defineEmits<{
  close: [];
}>();

const detail = useTaskDetailStore();
const ui = useUiStore();

async function onAdd(body: components['schemas']['CreateReferenceRequest']): Promise<void> {
  const ok = await detail.addReference(props.ws, props.readableId, body);

  if (ok) {
    ui.showBanner('Reference added', 'success');
    emit('close');
  } else if (detail.error) {
    ui.showBanner(detail.error, 'error');
  }
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') emit('close');
}

onMounted(() => window.addEventListener('keydown', onKeydown));
onUnmounted(() => window.removeEventListener('keydown', onKeydown));
</script>

<template>
  <Teleport to="body">
    <div
      class="fixed inset-0 flex items-center justify-center"
      style="background: var(--c-overlay); z-index: 300;"
      @mousedown.self="emit('close')"
    >
      <div role="dialog" aria-label="Link or add dependency" class="atl-link-dialog" @mousedown.stop>
        <div class="atl-link-head">
          <Icon name="link" :size="15" style="color: var(--c-muted); flex: 0 0 auto;" />
          <span class="atl-link-title">Link or add dependency</span>
          <span style="flex: 1;" />
          <button
            type="button"
            class="atl-gbtn"
            style="width: 26px; height: 26px;"
            title="Close"
            aria-label="Close"
            @click="emit('close')"
          >
            <Icon name="x" :size="16" />
          </button>
        </div>

        <div class="atl-link-body">
          <ReferenceAdd :ws="ws" default-kind="blocks" @add="onAdd" />
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.atl-link-dialog {
  width: 460px;
  max-width: calc(100vw - 32px);
  background: var(--c-panel);
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg);
  box-shadow: var(--shadow-lg);
  font-family: var(--font-ui);
}

.atl-link-head {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 13px 16px;
  border-bottom: 1px solid var(--c-border);
}

.atl-link-title {
  font-size: var(--fs-lg);
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
}

.atl-link-body {
  padding: 14px 16px 18px;
}
</style>
