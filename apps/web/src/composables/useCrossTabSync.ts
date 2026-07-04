import { onScopeDispose } from 'vue';
import {
  TAGS_KEY as KNOWN_TAGS_STORAGE_KEY,
  STORAGE_KEY as LABEL_COLORS_STORAGE_KEY,
  useLabelColorsStore,
} from '@/stores/labelColors';
import {
  EDITOR_WIDE_STORAGE_KEY,
  TASK_VIEW_MODE_STORAGE_KEY,
  THEME_STORAGE_KEY,
  useUiStore,
} from '@/stores/ui';

/**
 * Keeps app-global preferences in sync across same-user browser tabs.
 *
 * The native `storage` event fires in every OTHER tab when localStorage changes,
 * which is exactly the reach we need for local-only preference state that never
 * round-trips the server (theme, editor width, task-view mode, label colors,
 * known tags). Server-backed state is already synced by the SSE live-updates
 * layer and is intentionally out of scope here.
 *
 * Incoming values are routed to each store's `applyExternal*` method, which
 * updates reactive state WITHOUT re-persisting — preventing a write → `storage`
 * event → write feedback loop between tabs. Only the raw `event.newValue` is
 * used; localStorage is never re-read.
 *
 * Contextual/per-tab state (active workspace, route, open note tabs, filters,
 * sidebar/inspector/panel layout) is deliberately not synced.
 */
export function useCrossTabSync(): void {
  if (typeof window === 'undefined') return;

  const ui = useUiStore();
  const labelColors = useLabelColorsStore();

  function onStorage(event: StorageEvent): void {
    const value = event.newValue;

    // A null value means the key was removed; leave current state as-is.
    if (value === null) return;

    switch (event.key) {
      case THEME_STORAGE_KEY:
        ui.applyExternalTheme(value);
        break;
      case EDITOR_WIDE_STORAGE_KEY:
        ui.applyExternalEditorWide(value);
        break;
      case TASK_VIEW_MODE_STORAGE_KEY:
        ui.applyExternalTaskViewMode(value);
        break;
      case LABEL_COLORS_STORAGE_KEY:
        labelColors.applyExternalColors(value);
        break;
      case KNOWN_TAGS_STORAGE_KEY:
        labelColors.applyExternalKnownTags(value);
        break;
    }
  }

  window.addEventListener('storage', onStorage);
  onScopeDispose(() => window.removeEventListener('storage', onStorage));
}
