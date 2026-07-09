<script setup lang="ts">
import { computed } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import Kbd from '@/components/ui/Kbd.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import { useOverlayEscape } from '@/composables/useOverlayEscape';
import { getShortcutCatalog, type ShortcutMeta, type ShortcutScope } from '@/lib/keymap';

const props = defineProps<{
  open: boolean;
}>();

const emit = defineEmits<{
  close: [];
}>();

const scopeLabels: Record<ShortcutScope, string> = {
  global: 'Global',
  board: 'Board',
  task: 'Task detail',
  overlay: 'Overlay',
};

const shortcutsByScope = computed(() => {
  const groups = new Map<ShortcutScope, ShortcutMeta[]>();
  for (const shortcut of getShortcutCatalog()) {
    const entries = groups.get(shortcut.scope) ?? [];
    entries.push(shortcut);
    groups.set(shortcut.scope, entries);
  }
  return Array.from(groups.entries());
});

function displayKey(token: string): string {
  return token
    .replace('mod', '⌘/Ctrl')
    .replace('shift', 'Shift')
    .replace('escape', 'Esc')
    .replace(/\b([a-z])\b/g, (letter) => letter.toUpperCase());
}

useOverlayEscape(
  () => props.open,
  () => emit('close'),
);
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="fixed inset-0 flex items-center justify-center"
      style="background: var(--c-overlay); z-index: 320; padding: 24px;"
      @mousedown.self="emit('close')"
    >
      <section
        role="dialog"
        aria-modal="true"
        aria-labelledby="shortcuts-help-title"
        class="atl-shortcuts flex flex-col"
      >
        <header class="atl-shortcuts-head">
          <span class="atl-shortcuts-icon" aria-hidden="true">
            <Icon name="keyboard" :size="17" />
          </span>
          <div class="min-w-0 flex-1">
            <h2 id="shortcuts-help-title" class="atl-shortcuts-title">Keyboard shortcuts</h2>
            <p class="atl-shortcuts-subtitle">Fast paths available in this version of Atlas.</p>
          </div>
          <button
            type="button"
            class="atl-gbtn"
            style="width: 28px; height: 28px;"
            title="Close"
            aria-label="Close keyboard shortcuts"
            @click="emit('close')"
          >
            <Icon name="x" :size="16" />
          </button>
        </header>

        <div class="atl-shortcuts-body">
          <section v-for="[scope, shortcuts] in shortcutsByScope" :key="scope" class="atl-shortcuts-group">
            <SectionLabel>{{ scopeLabels[scope] }}</SectionLabel>
            <div class="atl-shortcuts-list">
              <div v-for="shortcut in shortcuts" :key="shortcut.id" class="atl-shortcuts-row">
                <span class="atl-shortcuts-label">{{ shortcut.label }}</span>
                <span class="atl-shortcuts-keys">
                  <Kbd v-for="key in shortcut.keys" :key="key">{{ displayKey(key) }}</Kbd>
                </span>
              </div>
            </div>
          </section>
        </div>

        <footer class="atl-shortcuts-foot">
          <Kbd>Esc</Kbd>
          closes this dialog
        </footer>
      </section>
    </div>
  </Teleport>
</template>

<style scoped>
.atl-shortcuts {
  width: 520px;
  max-width: 100%;
  max-height: min(680px, calc(100vh - 48px));
  background: var(--c-panel);
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
  font-family: var(--font-ui);
}

.atl-shortcuts-head {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  padding: 16px;
  border-bottom: 1px solid var(--c-border);
}

.atl-shortcuts-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--r-md);
  color: var(--c-primary);
  background: var(--c-selection);
  border: 1px solid var(--c-border);
  flex: 0 0 auto;
}

.atl-shortcuts-title {
  margin: 0;
  color: var(--c-foreground);
  font-size: var(--fs-xl);
  font-weight: var(--fw-bold);
}

.atl-shortcuts-subtitle {
  margin: 4px 0 0;
  color: var(--c-muted);
  font-size: var(--fs-sm);
  line-height: 1.45;
}

.atl-shortcuts-body {
  overflow-y: auto;
  padding: 8px 0;
}

.atl-shortcuts-group + .atl-shortcuts-group {
  margin-top: 6px;
}

.atl-shortcuts-list {
  padding: 0 8px 4px;
}

.atl-shortcuts-row {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 38px;
  padding: 6px 8px;
  border-radius: var(--r-md);
}

.atl-shortcuts-row:hover {
  background: var(--c-selection);
}

.atl-shortcuts-label {
  flex: 1;
  min-width: 0;
  color: var(--c-foreground);
  font-size: var(--fs-sm);
}

.atl-shortcuts-keys {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: 5px;
  flex: 0 0 auto;
}

.atl-shortcuts-foot {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 10px 16px;
  border-top: 1px solid var(--c-border);
  color: var(--c-muted);
  font-size: var(--fs-sm);
}
</style>
