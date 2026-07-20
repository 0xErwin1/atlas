import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const route = vi.hoisted(() => ({ params: {} as Record<string, string> }));
const router = vi.hoisted(() => ({ push: vi.fn() }));

vi.mock('vue-router', () => ({
  useRoute: () => route,
  useRouter: () => router,
}));

import { useDocsTabs } from '@/composables/useDocsTabs';
import { useNotesTabsStore } from '@/stores/notesTabs';
import { useWorkspaceStore } from '@/stores/workspace';

function seedTabs() {
  const store = useNotesTabsStore();
  store.open('ws', { kind: 'doc', id: 'note-a' }, 'Note A');
  store.open('ws', { kind: 'board', id: 'board-1' }, 'Board One');
  return store;
}

describe('useDocsTabs', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    localStorage.clear();
    route.params = {};
    router.push.mockReset();
    useWorkspaceStore().setActiveWorkspace('ws');
  });

  it('maps document and board tabs to the strip view model with their icons', () => {
    seedTabs();
    const { tabs } = useDocsTabs();

    expect(tabs.value).toEqual([
      { id: 'doc:note-a', name: 'Note A', icon: 'file', active: false, dirty: false },
      { id: 'board:board-1', name: 'Board One', icon: 'columns-3', active: false, dirty: false },
    ]);
  });

  it('marks the document tab active on a notes route', () => {
    seedTabs();
    route.params = { slug: 'note-a' };
    const { tabs } = useDocsTabs();

    expect(tabs.value.find((t) => t.id === 'doc:note-a')?.active).toBe(true);
    expect(tabs.value.find((t) => t.id === 'board:board-1')?.active).toBe(false);
  });

  it('marks the board tab active on a tasks route', () => {
    seedTabs();
    route.params = { boardId: 'board-1' };
    const { tabs } = useDocsTabs();

    expect(tabs.value.find((t) => t.id === 'board:board-1')?.active).toBe(true);
  });

  it('reflects the dirty document marker only on document tabs', () => {
    const store = seedTabs();
    store.setDirtyDoc('ws', 'note-a');
    const { tabs } = useDocsTabs();

    expect(tabs.value.find((t) => t.id === 'doc:note-a')?.dirty).toBe(true);
  });

  it('routes a document tab to notes and a board tab to tasks on select', () => {
    seedTabs();
    const { onSelect } = useDocsTabs();

    onSelect('doc:note-a');
    expect(router.push).toHaveBeenLastCalledWith({ name: 'notes', params: { slug: 'note-a' } });

    onSelect('board:board-1');
    expect(router.push).toHaveBeenLastCalledWith({ name: 'tasks', params: { boardId: 'board-1' } });
  });

  it('does not navigate when selecting the already-active tab', () => {
    seedTabs();
    route.params = { boardId: 'board-1' };
    const { onSelect } = useDocsTabs();

    onSelect('board:board-1');
    expect(router.push).not.toHaveBeenCalled();
  });

  it('navigates to the neighbour when the active tab is closed', () => {
    seedTabs();
    route.params = { slug: 'note-a' };
    const { onClose } = useDocsTabs();

    onClose('doc:note-a');
    expect(router.push).toHaveBeenLastCalledWith({ name: 'tasks', params: { boardId: 'board-1' } });
  });

  it('does not navigate when a background tab is closed', () => {
    seedTabs();
    route.params = { slug: 'note-a' };
    const { onClose } = useDocsTabs();

    onClose('board:board-1');
    expect(router.push).not.toHaveBeenCalled();
  });

  it('lands on the notes root when the last tab is closed', () => {
    const store = useNotesTabsStore();
    store.open('ws', { kind: 'board', id: 'board-1' }, 'Board One');
    route.params = { boardId: 'board-1' };
    const { onClose } = useDocsTabs();

    onClose('board:board-1');
    expect(router.push).toHaveBeenLastCalledWith({ name: 'notes' });
  });
});
