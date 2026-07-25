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
 * Desktop > App settings. The stored value is the source of truth: each control
 * only reflects what the host reported back, so a rejected change leaves the
 * control in agreement with the host.
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
const startOnLogin = ref(false);
const systemTray = ref(true);
const error = ref<string | null>(null);
const saving = ref(false);
const trayRestartRequired = ref(false);

const selected = computed(() => (decorations.value ? DECORATIONS_ON : DECORATIONS_OFF));

const zoomPercent = computed(() => `${Math.round(zoom.value * 100)}%`);
const canZoomIn = computed(() => zoom.value < MAX_ZOOM_FACTOR);
const canZoomOut = computed(() => zoom.value > MIN_ZOOM_FACTOR);
const canResetZoom = computed(() => zoom.value !== DEFAULT_ZOOM_FACTOR);

const FALLBACK_ERROR = 'Unable to change the window decorations';
const ZOOM_FALLBACK_ERROR = 'Unable to change the zoom level';
const START_ON_LOGIN_FALLBACK_ERROR = 'Unable to change the start on login setting';
const SYSTEM_TRAY_FALLBACK_ERROR = 'Unable to change the system tray setting';

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

void transport
  .getStartOnLogin()
  .then((result) => {
    if (result.data !== undefined) startOnLogin.value = result.data.start_on_login;
  })
  .catch(() => {
    startOnLogin.value = false;
  });

void transport
  .getSystemTray()
  .then((result) => {
    if (result.data !== undefined) systemTray.value = result.data.system_tray;
  })
  .catch(() => {
    systemTray.value = true;
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

function checked(event: Event): boolean | null {
  return event.target instanceof HTMLInputElement ? event.target.checked : null;
}

function restoreChecked(event: Event, value: boolean): void {
  if (event.target instanceof HTMLInputElement) event.target.checked = value;
}

async function updateStartOnLogin(event: Event): Promise<void> {
  const next = checked(event);
  if (next === null || next === startOnLogin.value || saving.value) return;

  error.value = null;
  saving.value = true;

  try {
    const result = await transport.setStartOnLogin(next);

    if (result.error || result.data === undefined) {
      error.value = typeof result.error === 'string' ? result.error : START_ON_LOGIN_FALLBACK_ERROR;
      return;
    }

    startOnLogin.value = result.data.start_on_login;
  } catch {
    error.value = START_ON_LOGIN_FALLBACK_ERROR;
  } finally {
    restoreChecked(event, startOnLogin.value);
    saving.value = false;
  }
}

async function updateSystemTray(event: Event): Promise<void> {
  const next = checked(event);
  if (next === null || next === systemTray.value || saving.value) return;

  error.value = null;
  saving.value = true;

  try {
    const result = await transport.setSystemTray(next);

    if (result.error || result.data === undefined) {
      error.value = typeof result.error === 'string' ? result.error : SYSTEM_TRAY_FALLBACK_ERROR;
      return;
    }

    systemTray.value = result.data.system_tray;
    trayRestartRequired.value = true;
  } catch {
    error.value = SYSTEM_TRAY_FALLBACK_ERROR;
  } finally {
    restoreChecked(event, systemTray.value);
    saving.value = false;
  }
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
        <div class="atl-pref-label">Start on login</div>
        <div class="atl-pref-hint">Launch Atlas Desktop when you sign in to this machine.</div>
      </div>
      <input
        aria-label="Start on login"
        type="checkbox"
        :checked="startOnLogin"
        :disabled="saving"
        @change="updateStartOnLogin"
      />
    </div>

    <div class="atl-pref-row">
      <div class="atl-pref-text">
        <div class="atl-pref-label">Show system tray icon</div>
        <div class="atl-pref-hint">Keep Atlas Desktop available from the system tray.</div>
        <div v-if="trayRestartRequired" class="atl-pref-hint">Restart Atlas Desktop to apply this change.</div>
      </div>
      <input
        aria-label="Show system tray icon"
        type="checkbox"
        :checked="systemTray"
        :disabled="saving"
        @change="updateSystemTray"
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
