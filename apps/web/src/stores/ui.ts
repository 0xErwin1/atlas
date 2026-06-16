import { defineStore } from 'pinia';
import { ref } from 'vue';

export type InspectorTab = 'properties' | 'backlinks' | 'activity' | 'share';
export type BannerType = 'error' | 'warning' | 'info' | 'success';

export interface Banner {
  message: string;
  type: BannerType;
}

const INSPECTOR_STORAGE_KEY = 'atlas:inspector';

function loadInspectorState(): { open: boolean; tab: InspectorTab } {
  try {
    const raw = localStorage.getItem(INSPECTOR_STORAGE_KEY);
    if (raw) return JSON.parse(raw) as { open: boolean; tab: InspectorTab };
  } catch {
    // ignore malformed storage
  }
  return { open: false, tab: 'properties' };
}

export const useUiStore = defineStore('ui', () => {
  const saved = loadInspectorState();

  const inspectorOpen = ref(saved.open);
  const inspectorTab = ref<InspectorTab>(saved.tab);
  const banner = ref<Banner | null>(null);

  const shareOpen = ref(false);
  const shareResourceLabel = ref('');

  function persistInspector() {
    try {
      localStorage.setItem(
        INSPECTOR_STORAGE_KEY,
        JSON.stringify({ open: inspectorOpen.value, tab: inspectorTab.value }),
      );
    } catch {
      // ignore storage errors
    }
  }

  function toggleInspector() {
    inspectorOpen.value = !inspectorOpen.value;
    persistInspector();
  }

  function setInspectorTab(tab: InspectorTab) {
    inspectorTab.value = tab;
    persistInspector();
  }

  function showBanner(message: string, type: BannerType) {
    banner.value = { message, type };
  }

  function dismissBanner() {
    banner.value = null;
  }

  function openShare(resourceLabel: string) {
    shareResourceLabel.value = resourceLabel;
    shareOpen.value = true;
  }

  function closeShare() {
    shareOpen.value = false;
  }

  return {
    inspectorOpen,
    inspectorTab,
    banner,
    shareOpen,
    shareResourceLabel,
    toggleInspector,
    setInspectorTab,
    showBanner,
    dismissBanner,
    openShare,
    closeShare,
  };
});
