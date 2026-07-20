import { type ComputedRef, computed } from 'vue';
import { useRouter } from 'vue-router';
import type { Tab } from '@/components/ui/TabStrip.vue';
import { routeAfterClose, routeForTab } from '@/lib/docsTabs';
import { parseTabKey, type TabRef, tabKey, useNotesTabsStore } from '@/stores/notesTabs';
import { useWorkspaceStore } from '@/stores/workspace';
import { useActiveSidebarNode } from './useActiveSidebarNode';

const TAB_ICON: Record<TabRef['kind'], string> = {
  doc: 'file',
  board: 'columns-3',
};

/**
 * Drives the workspace-level tab strip: it maps the persisted document/board
 * tabs to the strip's view model and turns strip events into navigation. Lives
 * at the shell level so the strip persists across the notes and tasks routes,
 * while the active tab is derived from the route (via `useActiveSidebarNode`) so
 * the highlight always agrees with the sidebar.
 */
export function useDocsTabs(): {
  tabs: ComputedRef<Tab[]>;
  onSelect: (key: string) => void;
  onClose: (key: string) => void;
  onCloseOthers: (key: string) => void;
  onCloseRight: (key: string) => void;
  onCloseAll: () => void;
} {
  const router = useRouter();
  const workspace = useWorkspaceStore();
  const store = useNotesTabsStore();
  const { activeSlug, activeBoardId } = useActiveSidebarNode();

  const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

  // The tab the current route selects, or null on a route that owns no tab (the
  // notes root, or a task view). Task detail resolves to its parent board via
  // `activeBoardId`, so an open board tab stays highlighted while drilling in.
  const activeRef = computed<TabRef | null>(() => {
    if (activeSlug.value !== null) return { kind: 'doc', id: activeSlug.value };
    if (activeBoardId.value !== null) return { kind: 'board', id: activeBoardId.value };
    return null;
  });

  function isActive(ref: TabRef): boolean {
    const active = activeRef.value;
    return active !== null && active.kind === ref.kind && active.id === ref.id;
  }

  const tabs = computed<Tab[]>(() =>
    store.tabs(ws.value).map((tab) => ({
      id: tabKey(tab),
      name: tab.title || 'Untitled',
      icon: TAB_ICON[tab.kind],
      active: isActive(tab),
      dirty: tab.kind === 'doc' && store.isDirtyDoc(ws.value, tab.id),
    })),
  );

  function onSelect(key: string): void {
    const ref = parseTabKey(key);
    if (!isActive(ref)) void router.push(routeForTab(ref));
  }

  function onClose(key: string): void {
    const ref = parseTabKey(key);
    const wasActive = isActive(ref);
    const next = store.close(ws.value, ref);
    if (wasActive) void router.push(routeAfterClose(next));
  }

  // After a bulk close, navigate only when the active tab was among those
  // removed; `survivor` is the tab to fall back to (null = the notes root).
  function navigateAfterClose(survivor: TabRef | null): void {
    const active = activeRef.value;
    if (active === null) return;
    if (store.tabs(ws.value).some((t) => t.kind === active.kind && t.id === active.id)) return;
    void router.push(routeAfterClose(survivor));
  }

  function onCloseOthers(key: string): void {
    navigateAfterClose(store.closeOthers(ws.value, parseTabKey(key)));
  }

  function onCloseRight(key: string): void {
    navigateAfterClose(store.closeRight(ws.value, parseTabKey(key)));
  }

  function onCloseAll(): void {
    navigateAfterClose(store.closeAll(ws.value));
  }

  return { tabs, onSelect, onClose, onCloseOthers, onCloseRight, onCloseAll };
}
