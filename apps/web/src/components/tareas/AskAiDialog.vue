<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import Btn from '@/components/ui/Btn.vue';
import Icon from '@/components/ui/Icon.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import { useOverlayEscape } from '@/composables/useOverlayEscape';
import { AI_ACTIONS, type AiAction, type AiPromptTask, buildTaskAiPrompt } from '@/lib/aiPrompt';
import { useUiStore } from '@/stores/ui';

/**
 * Composes a ready-to-paste prompt from a task's details for a chosen assist
 * action and lets the user copy it to hand off to their own AI agent. Atlas runs
 * no model itself; this is a copy-prompt bridge, not a live completion. The
 * parent owns visibility via `open` and reacts to `close`.
 */
const props = defineProps<{
  open: boolean;
  task: AiPromptTask | null;
  statusName: string | null;
  action: AiAction;
}>();

const emit = defineEmits<{
  close: [];
}>();

const ui = useUiStore();
const { isMobile } = useBreakpoint();

const current = ref<AiAction>(props.action);
const extra = ref('');
const copied = ref(false);

watch(
  () => props.open,
  (open) => {
    if (open) {
      current.value = props.action;
      extra.value = '';
      copied.value = false;
    }
  },
);

function selectAction(action: AiAction): void {
  current.value = action;
  copied.value = false;
}

const prompt = computed(() =>
  props.task === null ? '' : buildTaskAiPrompt(props.task, props.statusName, current.value, extra.value),
);

async function copyPrompt(): Promise<void> {
  try {
    await navigator.clipboard.writeText(prompt.value);
    copied.value = true;
    setTimeout(() => {
      copied.value = false;
    }, 1500);
  } catch {
    ui.showBanner('Clipboard is not available', 'error');
  }
}

useOverlayEscape(
  () => props.open,
  () => emit('close'),
);
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open && task"
      class="fixed inset-0 flex justify-center"
      :class="isMobile ? 'items-end' : 'items-center'"
      style="background: var(--c-overlay); z-index: 300;"
      @mousedown.self="emit('close')"
    >
      <div
        role="dialog"
        aria-label="Ask AI"
        class="atl-askai"
        :class="{ mobile: isMobile }"
        @mousedown.stop
      >
        <div class="atl-askai-head">
          <Icon name="sparkles" :size="16" style="color: var(--c-agent); flex: 0 0 auto;" />
          <span class="atl-askai-title">Ask AI</span>
          <span style="flex: 1;" />
          <span class="atl-askai-id">{{ task.readable_id }}</span>
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

        <div class="atl-askai-body">
          <div class="atl-askai-actions">
            <button
              v-for="a in AI_ACTIONS"
              :key="a.value"
              type="button"
              class="atl-askai-tab"
              :class="{ active: a.value === current }"
              @click="selectAction(a.value)"
            >
              <Icon :name="a.icon" :size="14" />
              {{ a.label }}
            </button>
          </div>

          <div class="atl-askai-label">Prompt</div>
          <pre class="atl-askai-prompt">{{ prompt }}</pre>

          <div class="atl-askai-copyrow">
            <Btn variant="primary" @click="copyPrompt">
              <Icon :name="copied ? 'check' : 'copy'" :size="14" />
              {{ copied ? 'Copied' : 'Copy prompt' }}
            </Btn>
          </div>

          <div class="atl-askai-label" style="margin-top: 16px;">
            Give more detail <span style="color: var(--c-muted); text-transform: none; letter-spacing: 0;">(optional)</span>
          </div>
          <textarea
            v-model="extra"
            class="atl-askai-extra"
            rows="3"
            placeholder="Anything the AI should know — constraints, context, the outcome you want…"
          />
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.atl-askai {
  width: 600px;
  max-width: calc(100vw - 32px);
  max-height: 86vh;
  display: flex;
  flex-direction: column;
  background: var(--c-panel);
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg);
  box-shadow: var(--shadow-lg);
  font-family: var(--font-ui);
}

.atl-askai.mobile {
  width: 100%;
  max-width: 100%;
  max-height: 90vh;
  border-radius: var(--r-lg) var(--r-lg) 0 0;
  border-bottom: none;
}

.atl-askai-head {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 13px 16px;
  border-bottom: 1px solid var(--c-border);
}

.atl-askai-title {
  font-size: var(--fs-xl);
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
}

.atl-askai-id {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-askai-body {
  padding: 16px;
  overflow-y: auto;
}

.atl-askai-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 16px;
}

.atl-askai-tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 10px;
  background: var(--c-secondary);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  color: var(--c-foreground);
  font-family: var(--font-ui);
  font-size: var(--fs-sm);
  cursor: pointer;
  transition:
    background-color 0.12s ease,
    border-color 0.12s ease;
}

.atl-askai-tab:hover {
  border-color: var(--c-muted);
}

.atl-askai-tab.active {
  color: var(--c-agent);
  background: var(--c-agent-bg);
  border-color: var(--c-agent-border);
}

.atl-askai-label {
  font-size: var(--fs-xs);
  font-weight: var(--fw-semibold);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--c-muted);
  margin-bottom: 6px;
}

.atl-askai-prompt {
  margin: 0;
  max-height: 260px;
  overflow: auto;
  padding: 12px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  color: var(--c-foreground);
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  line-height: var(--lh-normal);
  white-space: pre-wrap;
  word-break: break-word;
}

.atl-askai-copyrow {
  display: flex;
  margin-top: 10px;
}

.atl-askai-extra {
  width: 100%;
  resize: vertical;
  padding: 8px 10px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  color: var(--c-foreground);
  font-family: var(--font-ui);
  font-size: var(--fs-sm);
  line-height: var(--lh-normal);
  outline: none;
  transition:
    border-color 0.12s ease,
    box-shadow 0.12s ease;
}

.atl-askai-extra:hover {
  border-color: var(--c-muted);
}

.atl-askai-extra:focus {
  border-color: var(--c-primary);
}
</style>
