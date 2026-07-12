import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('vue-router', () => ({
  createRouter: () => ({ beforeEach: vi.fn(), afterEach: vi.fn() }),
  createWebHistory: () => ({}),
}));

import { recordLastViewed } from '@/router';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useWorkspaceStore } from '@/stores/workspace';

describe('recordLastViewed guard', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    try {
      localStorage.clear();
    } catch {
      // jsdom provides localStorage; ignore if absent
    }
  });

  it('does not record while a workspace switch is in progress', () => {
    const workspace = useWorkspaceStore();
    workspace.setActiveWorkspace('atlas');
    workspace.beginSwitch();

    recordLastViewed({ name: 'notes', params: { slug: 'restored-in-personal' } });

    expect(useLastViewedStore().forWorkspace('atlas')).toBeNull();
  });

  it('records under the destination workspace once the switch has ended', () => {
    const workspace = useWorkspaceStore();
    workspace.beginSwitch();
    workspace.setActiveWorkspace(null);

    // Mid-switch navigation to the restored resource must not be recorded.
    recordLastViewed({ name: 'notes', params: { slug: 'restored-note' } });

    workspace.endSwitch();
    workspace.switchWorkspace('personal');

    // A normal navigation after the switch records under the destination slug.
    recordLastViewed({ name: 'notes', params: { slug: 'restored-note' } });

    const lastViewed = useLastViewedStore();
    expect(lastViewed.forWorkspace('personal')).toEqual({
      name: 'notes',
      params: { slug: 'restored-note' },
    });
    expect(lastViewed.forWorkspace('atlas')).toBeNull();
  });

  it('ignores paramless section roots', () => {
    const workspace = useWorkspaceStore();
    workspace.setActiveWorkspace('atlas');

    recordLastViewed({ name: 'notes', params: {} });

    expect(useLastViewedStore().forWorkspace('atlas')).toBeNull();
  });
});
