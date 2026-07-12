import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { isNavigationFailure, NavigationFailureType, routeState, push } = vi.hoisted(() => ({
  isNavigationFailure: vi.fn((_result: unknown, _type?: number) => false),
  NavigationFailureType: { redirected: 2, aborted: 4, cancelled: 8, duplicated: 16 },
  routeState: { name: 'notes' as string },
  push: vi.fn(),
}));

vi.mock('vue-router', () => ({
  isNavigationFailure,
  NavigationFailureType,
  useRoute: () => routeState,
  useRouter: () => ({ push }),
}));

import { useWorkspaceSwitch } from '@/composables/useWorkspaceSwitch';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useWorkspaceStore } from '@/stores/workspace';

describe('useWorkspaceSwitch', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    isNavigationFailure.mockReturnValue(false);
    routeState.name = 'notes';
    push.mockResolvedValue(undefined);
    try {
      localStorage.clear();
    } catch {
      // jsdom provides localStorage; ignore if absent
    }
  });

  it('restores the destination workspace last-viewed resource', async () => {
    const workspace = useWorkspaceStore();
    workspace.setActiveWorkspace('atlas');
    useLastViewedStore().record('personal', { name: 'tasks', params: { boardId: 'b1' } });

    const { switchTo } = useWorkspaceSwitch();
    await switchTo('personal');

    expect(push).toHaveBeenCalledWith({ name: 'tasks', params: { boardId: 'b1' } });
    expect(workspace.activeWorkspaceSlug).toBe('personal');
  });

  it('falls back to the current section root when the destination has no history', async () => {
    routeState.name = 'search';
    const workspace = useWorkspaceStore();
    workspace.setActiveWorkspace('atlas');

    const { switchTo } = useWorkspaceSwitch();
    await switchTo('personal');

    expect(push).toHaveBeenCalledWith({ name: 'search' });
  });

  it('is a no-op when switching to the already-active workspace', async () => {
    const workspace = useWorkspaceStore();
    workspace.setActiveWorkspace('atlas');

    const { switchTo } = useWorkspaceSwitch();
    await switchTo('atlas');

    expect(push).not.toHaveBeenCalled();
  });

  it('holds the switching flag until navigation resolves, then clears it', async () => {
    let finishNavigation: (() => void) | undefined;
    push.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finishNavigation = resolve;
        }),
    );
    const workspace = useWorkspaceStore();
    workspace.setActiveWorkspace('atlas');

    const { switchTo } = useWorkspaceSwitch();
    const pending = switchTo('personal');

    await vi.waitFor(() => expect(push).toHaveBeenCalled());
    expect(workspace.switching).toBe(true);
    expect(workspace.activeWorkspaceSlug).toBeNull();

    finishNavigation?.();
    await pending;

    expect(workspace.switching).toBe(false);
    expect(workspace.activeWorkspaceSlug).toBe('personal');
  });

  it('reverts the active workspace and clears switching on an aborted navigation', async () => {
    const failure = { type: NavigationFailureType.aborted };
    push.mockResolvedValueOnce(failure);
    isNavigationFailure.mockImplementation(
      (result, type) => result === failure && type === NavigationFailureType.aborted,
    );
    const workspace = useWorkspaceStore();
    workspace.setActiveWorkspace('atlas');

    const { switchTo } = useWorkspaceSwitch();
    await switchTo('personal');

    expect(workspace.activeWorkspaceSlug).toBe('atlas');
    expect(workspace.switching).toBe(false);
  });
});
