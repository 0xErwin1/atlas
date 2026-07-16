<script setup lang="ts">
import { computed, ref } from 'vue';
import PanelHeader from '@/components/settings/PanelHeader.vue';
import SegmentedControl, { type SegmentedOption } from '@/components/ui/SegmentedControl.vue';
import { getPlatformTransport } from '@/platform/transport';

/**
 * Desktop > App settings. Exposes the machine-local window-decorations
 * preference. The stored value is the source of truth: the segmented control
 * only ever reflects what the host reported back, so a rejected change leaves
 * the window and the control in agreement.
 */

const DECORATIONS_ON = 'on';
const DECORATIONS_OFF = 'off';

const DECORATION_OPTIONS: SegmentedOption[] = [
  { value: DECORATIONS_ON, label: 'On', icon: 'app-window' },
  { value: DECORATIONS_OFF, label: 'Off', icon: 'square' },
];

const transport = getPlatformTransport();

const decorations = ref(true);
const error = ref<string | null>(null);
const saving = ref(false);

const selected = computed(() => (decorations.value ? DECORATIONS_ON : DECORATIONS_OFF));

void transport.getWindowDecorations().then((result) => {
  if (result.data !== undefined) decorations.value = result.data.window_decorations;
});

async function selectDecorations(value: string): Promise<void> {
  const next = value === DECORATIONS_ON;
  if (next === decorations.value || saving.value) return;

  error.value = null;
  saving.value = true;
  const result = await transport.setWindowDecorations(next);
  saving.value = false;

  if (result.data === undefined) {
    error.value = 'Unable to change the window decorations';
    return;
  }

  decorations.value = result.data.window_decorations;
}
</script>

<template>
  <div>
    <PanelHeader
      title="App settings"
      subtitle="Preferences for this machine — they are not synced to your account"
    />

    <div class="atl-pref-row">
      <div class="atl-pref-text">
        <div class="atl-pref-label">Window decorations</div>
        <div class="atl-pref-hint">
          Show the system title bar and window controls. Turn this off for a borderless window.
        </div>
      </div>
      <SegmentedControl
        :model-value="selected"
        :options="DECORATION_OPTIONS"
        @update:model-value="selectDecorations"
      />
    </div>

    <div v-if="error" class="atl-pref-error">{{ error }}</div>
  </div>
</template>

<style scoped>
.atl-pref-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  max-width: 560px;
}

.atl-pref-text {
  min-width: 0;
}

.atl-pref-label {
  font-size: 13px;
  font-weight: var(--fw-medium);
  color: var(--c-foreground);
}

.atl-pref-hint {
  font-size: 12px;
  color: var(--c-muted);
  margin-top: 3px;
}

.atl-pref-error {
  font-size: 12px;
  color: var(--c-danger);
  margin-top: 12px;
}
</style>
