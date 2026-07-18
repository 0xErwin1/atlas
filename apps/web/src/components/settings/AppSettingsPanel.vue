<script setup lang="ts">
import { computed, ref } from 'vue';
import PanelHeader from '@/components/settings/PanelHeader.vue';
import Btn from '@/components/ui/Btn.vue';
import Icon from '@/components/ui/Icon.vue';
import SegmentedControl, { type SegmentedOption } from '@/components/ui/SegmentedControl.vue';
import {
  DEFAULT_ZOOM_FACTOR,
  getPlatformTransport,
  MAX_ZOOM_FACTOR,
  MIN_ZOOM_FACTOR,
  ZOOM_FACTOR_STEP,
} from '@/platform/transport';

/**
 * Desktop > App settings. Exposes the machine-local window-decorations and zoom
 * preferences. The stored value is the source of truth: each control only ever
 * reflects what the host reported back, so a rejected change leaves the window
 * and the control in agreement.
 */

const DECORATIONS_ON = 'on';
const DECORATIONS_OFF = 'off';

const DECORATION_OPTIONS: SegmentedOption[] = [
  { value: DECORATIONS_ON, label: 'On', icon: 'app-window' },
  { value: DECORATIONS_OFF, label: 'Off', icon: 'square' },
];

const transport = getPlatformTransport();

const decorations = ref(true);
const zoom = ref(DEFAULT_ZOOM_FACTOR);
const error = ref<string | null>(null);
const saving = ref(false);

const selected = computed(() => (decorations.value ? DECORATIONS_ON : DECORATIONS_OFF));

const zoomPercent = computed(() => `${Math.round(zoom.value * 100)}%`);
const canZoomIn = computed(() => zoom.value < MAX_ZOOM_FACTOR);
const canZoomOut = computed(() => zoom.value > MIN_ZOOM_FACTOR);
const canResetZoom = computed(() => zoom.value !== DEFAULT_ZOOM_FACTOR);

const FALLBACK_ERROR = 'Unable to change the window decorations';
const ZOOM_FALLBACK_ERROR = 'Unable to change the zoom level';

void transport
  .getWindowDecorations()
  .then((result) => {
    if (result.data !== undefined) decorations.value = result.data.window_decorations;
  })
  .catch(() => {
    decorations.value = true;
  });

void transport
  .getZoom()
  .then((result) => {
    if (result.data !== undefined) zoom.value = result.data.zoom_factor;
  })
  .catch(() => {
    zoom.value = DEFAULT_ZOOM_FACTOR;
  });

async function selectDecorations(value: string): Promise<void> {
  const next = value === DECORATIONS_ON;
  if (next === decorations.value || saving.value) return;

  error.value = null;
  saving.value = true;

  try {
    const result = await transport.setWindowDecorations(next);

    if (result.error || result.data === undefined) {
      error.value = typeof result.error === 'string' ? result.error : FALLBACK_ERROR;
      return;
    }

    decorations.value = result.data.window_decorations;
  } catch {
    error.value = FALLBACK_ERROR;
  } finally {
    saving.value = false;
  }
}

function clampZoom(value: number): number {
  return Math.min(MAX_ZOOM_FACTOR, Math.max(MIN_ZOOM_FACTOR, value));
}

async function applyZoom(next: number): Promise<void> {
  const target = clampZoom(next);
  if (target === zoom.value || saving.value) return;

  error.value = null;
  saving.value = true;

  try {
    const result = await transport.setZoom(target);

    if (result.error || result.data === undefined) {
      error.value = typeof result.error === 'string' ? result.error : ZOOM_FALLBACK_ERROR;
      return;
    }

    zoom.value = result.data.zoom_factor;
  } catch {
    error.value = ZOOM_FALLBACK_ERROR;
  } finally {
    saving.value = false;
  }
}

function zoomIn(): void {
  void applyZoom(zoom.value + ZOOM_FACTOR_STEP);
}

function zoomOut(): void {
  void applyZoom(zoom.value - ZOOM_FACTOR_STEP);
}

function resetZoom(): void {
  void applyZoom(DEFAULT_ZOOM_FACTOR);
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

    <div class="atl-pref-row">
      <div class="atl-pref-text">
        <div class="atl-pref-label">Zoom</div>
        <div class="atl-pref-hint">
          Scale the whole interface. Also adjustable with Ctrl or Cmd and the plus, minus, or zero keys.
        </div>
      </div>
      <div class="atl-zoom-control">
        <Btn variant="ghost" aria-label="Zoom out" :disabled="!canZoomOut || saving" @click="zoomOut">
          <Icon name="minus" />
        </Btn>
        <span class="atl-zoom-value">{{ zoomPercent }}</span>
        <Btn variant="ghost" aria-label="Zoom in" :disabled="!canZoomIn || saving" @click="zoomIn">
          <Icon name="plus" />
        </Btn>
        <Btn variant="secondary" :disabled="!canResetZoom || saving" @click="resetZoom"> Reset </Btn>
      </div>
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

.atl-pref-row + .atl-pref-row {
  margin-top: 20px;
}

.atl-zoom-control {
  display: flex;
  align-items: center;
  gap: 8px;
}

.atl-zoom-value {
  min-width: 48px;
  text-align: center;
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  color: var(--c-foreground);
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
