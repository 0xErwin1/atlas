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

  it('a superseded switch neither reverts nor commits after a newer switch committed', async () => {
    const abortedFirst = { type: NavigationFailureType.aborted };
    let resolveFirst: ((value: unknown) => void) | undefined;
    push
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveFirst = resolve;
          }),
      )
      .mockResolvedValueOnce(undefined);
    isNavigationFailure.mockImplementation(
      (result, type) => result === abortedFirst && type === NavigationFailureType.aborted,
    );

    const workspace = useWorkspaceStore();
    workspace.setActiveWorkspace('atlas');

    const { switchTo } = useWorkspaceSwitch();
    const pendingFirst = switchTo('workspace-a');
    const pendingSecond = switchTo('personal');

    await pendingSecond;
    expect(workspace.activeWorkspaceSlug).toBe('personal');

    resolveFirst?.(abortedFirst);
    await pendingFirst;

    expect(workspace.activeWorkspaceSlug).toBe('personal');
    expect(workspace.switching).toBe(false);
  });

  it('a superseded switch that navigates successfully does not clobber the newer committed slug', async () => {
    let resolveFirst: ((value: unknown) => void) | undefined;
    push
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveFirst = resolve;
          }),
      )
      .mockResolvedValueOnce(undefined);

    const workspace = useWorkspaceStore();
    workspace.setActiveWorkspace('atlas');

    const { switchTo } = useWorkspaceSwitch();
    const pendingFirst = switchTo('workspace-a');
    const pendingSecond = switchTo('personal');

    await pendingSecond;
    expect(workspace.activeWorkspaceSlug).toBe('personal');

    // The older switch now resolves with a SUCCESSFUL navigation. Without the
    // isCurrentSwitch guard on the final commit it would call switchWorkspace and
    // overwrite the newer workspace with its own stale target.
    resolveFirst?.(undefined);
    await pendingFirst;

    expect(workspace.activeWorkspaceSlug).toBe('personal');
    expect(workspace.switching).toBe(false);
  });

  it('reverts the newest aborted switch to the committed workspace, not a transient null', async () => {
    const abortedSecond = { type: NavigationFailureType.aborted };
    let resolveFirst: ((value: unknown) => void) | undefined;
    push
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveFirst = resolve;
          }),
      )
      .mockResolvedValueOnce(abortedSecond);
    isNavigationFailure.mockImplementation(
      (result, type) => result === abortedSecond && type === NavigationFailureType.aborted,
    );

    const workspace = useWorkspaceStore();
    workspace.setActiveWorkspace('atlas');

    const { switchTo } = useWorkspaceSwitch();
    const pendingFirst = switchTo('workspace-a');
    const pendingSecond = switchTo('personal');

    await pendingSecond;

    // The earlier switch already nulled the active slug before this newest switch
    // ran, so a per-call snapshot would revert to null. It must revert to the last
    // committed workspace instead.
    expect(workspace.activeWorkspaceSlug).toBe('atlas');

    resolveFirst?.(undefined);
    await pendingFirst;

    expect(workspace.activeWorkspaceSlug).toBe('atlas');
    expect(workspace.switching).toBe(false);
  });
});
