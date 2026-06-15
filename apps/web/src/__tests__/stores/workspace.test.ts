import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET: vi.fn(),
  },
}));

import { useWorkspaceStore } from '@/stores/workspace';

describe('useWorkspaceStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('starts with no active workspace', () => {
    const store = useWorkspaceStore();
    expect(store.activeWorkspaceSlug).toBeNull();
  });

  it('setActiveWorkspace updates slug (REQ-W12)', () => {
    const store = useWorkspaceStore();
    store.setActiveWorkspace('my-workspace');
    expect(store.activeWorkspaceSlug).toBe('my-workspace');
  });

  it('setActiveWorkspace replacing slug updates correctly', () => {
    const store = useWorkspaceStore();
    store.setActiveWorkspace('first');
    store.setActiveWorkspace('second');
    expect(store.activeWorkspaceSlug).toBe('second');
  });
});
