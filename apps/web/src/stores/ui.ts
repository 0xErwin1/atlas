import { defineStore } from 'pinia';
import { ref } from 'vue';

export type InspectorTab = 'properties' | 'backlinks' | 'activity' | 'share';
export type BannerType = 'error' | 'warning' | 'info' | 'success';
export type Theme = 'dark' | 'light';

export interface Banner {
  message: string;
  type: BannerType;
}

const INSPECTOR_STORAGE_KEY = 'atlas:inspector';
const EDITOR_WIDE_STORAGE_KEY = 'atlas:editor-wide';
const THEME_STORAGE_KEY = 'atlas:theme';

function loadTheme(): Theme {
  try {
    const v = localStorage.getItem(THEME_STORAGE_KEY);
    if (v === 'light' || v === 'dark') return v;
  } catch {
    // ignore malformed storage
  }
  return 'dark';
}

function applyTheme(theme: Theme): void {
  try {
    document.documentElement.dataset.theme = theme;
  } catch {
    // no document (non-browser context)
  }
}

function loadInspectorState(): { open: boolean; tab: InspectorTab } {
  try {
    const raw = localStorage.getItem(INSPECTOR_STORAGE_KEY);
    if (raw) return JSON.parse(raw) as { open: boolean; tab: InspectorTab };
  } catch {
    // ignore malformed storage
  }
  return { open: false, tab: 'properties' };
}

function loadEditorWide(): boolean {
  try {
    return localStorage.getItem(EDITOR_WIDE_STORAGE_KEY) === '1';
  } catch {
    return false;
  }
}

export const useUiStore = defineStore('ui', () => {
  const saved = loadInspectorState();

  const inspectorOpen = ref(saved.open);
  const inspectorTab = ref<InspectorTab>(saved.tab);
  const banner = ref<Banner | null>(null);

  const shareOpen = ref(false);
  const shareResourceLabel = ref('');

  // Editor reading width: false = readable column, true = full viewport width.
  const editorWide = ref(loadEditorWide());

  const theme = ref<Theme>(loadTheme());
  applyTheme(theme.value);

  function setTheme(next: Theme) {
    theme.value = next;
    applyTheme(next);
    try {
      localStorage.setItem(THEME_STORAGE_KEY, next);
    } catch {
      // ignore storage errors
    }
  }

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

  function toggleEditorWide() {
    editorWide.value = !editorWide.value;
    try {
      localStorage.setItem(EDITOR_WIDE_STORAGE_KEY, editorWide.value ? '1' : '0');
    } catch {
      // ignore storage errors
    }
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
    editorWide,
    toggleEditorWide,
    theme,
    setTheme,
    toggleInspector,
    setInspectorTab,
    showBanner,
    dismissBanner,
    openShare,
    closeShare,
  };
});
