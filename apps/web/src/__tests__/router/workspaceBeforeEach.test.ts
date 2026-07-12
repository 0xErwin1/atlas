import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('vue-router', () => ({
  createRouter: () => ({ beforeEach: vi.fn(), afterEach: vi.fn() }),
  createWebHistory: () => ({}),
}));

vi.mock('@/stores/auth', () => ({
  useAuthStore: () => ({ isAuthenticated: true, fetchMe: vi.fn() }),
}));

vi.mock('@/stores/uiState', () => ({
  useUiStateStore: () => ({ loaded: true, load: vi.fn() }),
}));

import type { RouteLocationNormalized } from 'vue-router';
import { workspaceBeforeEach } from '@/router';
import { useWorkspaceStore } from '@/stores/workspace';

const notesRoute = {
  name: 'notes',
  params: {},
  query: {},
  meta: {},
  fullPath: '/n',
} as unknown as RouteLocationNormalized;

describe('workspaceBeforeEach bootstrap guard', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('bootstraps workspaces when the active workspace is null and no switch is in flight', async () => {
    const workspace = useWorkspaceStore();
    const spy = vi.spyOn(workspace, 'loadWorkspaces').mockResolvedValue(null);

    await workspaceBeforeEach(notesRoute);

    expect(spy).toHaveBeenCalledTimes(1);
  });

  it('skips the bootstrap while a workspace switch is in progress', async () => {
    const workspace = useWorkspaceStore();
    const spy = vi.spyOn(workspace, 'loadWorkspaces').mockResolvedValue(null);
    workspace.beginSwitch();

    await workspaceBeforeEach(notesRoute);

    expect(spy).not.toHaveBeenCalled();
  });

  it('skips the bootstrap when a workspace is already active', async () => {
    const workspace = useWorkspaceStore();
    const spy = vi.spyOn(workspace, 'loadWorkspaces').mockResolvedValue(null);
    workspace.setActiveWorkspace('atlas');

    await workspaceBeforeEach(notesRoute);

    expect(spy).not.toHaveBeenCalled();
  });
});
