import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { go, isNavigationFailure, NavigationFailureType, routeState, push, replace } = vi.hoisted(() => ({
  go: vi.fn(),
  isNavigationFailure: vi.fn((_result: unknown, _type?: number) => false),
  NavigationFailureType: { redirected: 2, aborted: 4, cancelled: 8, duplicated: 16 },
  routeState: { name: 'notes' as string, fullPath: '/w/atlas/notes' },
  push: vi.fn(),
  replace: vi.fn(),
}));

vi.mock('vue-router', () => ({
  isNavigationFailure,
  NavigationFailureType,
  useRoute: () => routeState,
  useRouter: () => ({ go, push, replace }),
}));

import { configureResourceCacheForTest, setResourceCachePrincipal } from '@/cache/cacheRuntime';
import { ResourceCache } from '@/cache/resourceCache';
import AppRail from '@/components/shell/AppRail.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

async function switchToPersonal(): Promise<void> {
  const wrapper = mount(AppRail);
  await wrapper.get('[aria-label="Switch workspace"]').trigger('click');
  const personal = wrapper.findAll('.atl-account-item').find((b) => b.text().includes('Personal'));
  if (personal === undefined) throw new Error('Personal workspace item not rendered');
  await personal.trigger('click');
}

function seed() {
  const workspace = useWorkspaceStore();
  workspace.setActiveWorkspace('atlas');
  workspace.workspaces = [
    {
      id: '019ef171-bbcf-7b90-9be6-5dbb382afd08',
      name: 'Atlas',
      slug: 'atlas',
      created_at: 'x',
      updated_at: 'x',
    },
    {
      id: '019ef171-bbcf-7b90-9be6-5dbb382afd09',
      name: 'Personal',
      slug: 'personal',
      created_at: 'x',
      updated_at: 'x',
    },
  ];
  return workspace;
}

describe('AppRail workspace switcher', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    isNavigationFailure.mockReturnValue(false);
    routeState.name = 'notes';
    try {
      localStorage.clear();
    } catch {
      // jsdom provides localStorage; ignore if absent
    }
    configureResourceCacheForTest({
      allow: vi.fn(),
      block: vi.fn(),
      clear: vi.fn().mockResolvedValue(undefined),
      purge: vi.fn().mockResolvedValue(undefined),
      purgeTags: vi.fn().mockResolvedValue(undefined),
      purgeWorkspace: vi.fn().mockImplementation(async () => undefined),
    });
  });

  it('lists every workspace and a create action in the menu', async () => {
    seed();
    const wrapper = mount(AppRail);

    await wrapper.get('[aria-label="Switch workspace"]').trigger('click');

    const text = wrapper.text();
    expect(text).toContain('Atlas');
    expect(text).toContain('Personal');
    expect(text).toContain('New workspace');
  });

  it('switches the workspace when another one is picked', async () => {
    const workspace = seed();
    const spy = vi.spyOn(workspace, 'switchWorkspace');

    await switchToPersonal();

    expect(spy).toHaveBeenCalledWith('personal');
  });

  it.each([
    'tasks',
    'task-view',
    'task-detail',
  ])('keeps the user in Tasks after switching from %s (ATL-49)', async (routeName) => {
    routeState.name = routeName;
    seed();

    await switchToPersonal();

    expect(push).toHaveBeenCalledWith({ name: 'tasks' });
  });

  it('keeps the user in Search after switching from Search (ATL-49)', async () => {
    routeState.name = 'search';
    seed();

    await switchToPersonal();

    expect(push).toHaveBeenCalledWith({ name: 'search' });
  });

  it('lands on Notes after switching from a note route (ATL-49)', async () => {
    routeState.name = 'notes';
    seed();

    await switchToPersonal();

    expect(push).toHaveBeenCalledWith({ name: 'notes' });
  });

  it('drops the stale resource route before activating the destination workspace', async () => {
    let finishNavigation: (() => void) | undefined;
    push.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finishNavigation = resolve;
        }),
    );
    const workspace = seed();

    const switching = switchToPersonal();

    await vi.waitFor(() => expect(push).toHaveBeenCalledWith({ name: 'notes' }));
    expect(workspace.activeWorkspaceSlug).toBeNull();

    finishNavigation?.();
    await switching;

    expect(workspace.activeWorkspaceSlug).toBe('personal');
  });

  it('does not mount the Tasks root with the old workspace active', async () => {
    routeState.name = 'task-detail';
    const workspace = seed();
    push.mockImplementationOnce(async () => {
      expect(workspace.activeWorkspaceSlug).toBeNull();
    });

    await switchToPersonal();

    expect(workspace.activeWorkspaceSlug).toBe('personal');
  });

  it('keeps the old workspace active when navigation is aborted', async () => {
    const failure = { type: NavigationFailureType.aborted };
    push.mockResolvedValueOnce(failure);
    isNavigationFailure.mockImplementation(
      (result, type) => result === failure && type === NavigationFailureType.aborted,
    );
    const workspace = seed();
    const spy = vi.spyOn(workspace, 'switchWorkspace');

    await switchToPersonal();

    expect(spy).not.toHaveBeenCalled();
    expect(workspace.activeWorkspaceSlug).toBe('atlas');
  });

  it('activates the destination workspace when the restored route duplicates the current URL (ATL-77)', async () => {
    const failure = { type: NavigationFailureType.duplicated };
    push.mockResolvedValueOnce(failure);
    isNavigationFailure.mockImplementation((result, type) => result === failure && type === failure.type);
    const workspace = seed();
    const spy = vi.spyOn(workspace, 'switchWorkspace');

    await switchToPersonal();

    expect(spy).toHaveBeenCalledWith('personal');
    expect(workspace.activeWorkspaceSlug).toBe('personal');
  });

  it('clears the switching flag when navigation succeeds', async () => {
    const workspace = seed();

    await switchToPersonal();

    expect(workspace.switching).toBe(false);
    expect(workspace.activeWorkspaceSlug).toBe('personal');
  });

  it('clears the switching flag when navigation is aborted', async () => {
    const failure = { type: NavigationFailureType.aborted };
    push.mockResolvedValueOnce(failure);
    isNavigationFailure.mockImplementation(
      (result, type) => result === failure && type === NavigationFailureType.aborted,
    );
    const workspace = seed();

    await switchToPersonal();

    expect(workspace.switching).toBe(false);
  });

  it('clears the switching flag when navigation rejects', async () => {
    push.mockRejectedValueOnce(new Error('navigation rejected'));
    const workspace = seed();

    await switchToPersonal();
    await flushPromises();

    expect(workspace.switching).toBe(false);
  });

  it('restores the old workspace when navigation rejects', async () => {
    push.mockRejectedValueOnce(new Error('navigation rejected'));
    const workspace = seed();

    await switchToPersonal();
    await flushPromises();

    expect(workspace.activeWorkspaceSlug).toBe('atlas');
  });

  it('restores the last-viewed resource of the destination workspace (ATL-77)', async () => {
    seed();
    const lastViewed = useLastViewedStore();
    lastViewed.record('personal', { name: 'notes', params: { slug: 'restored-note' } });

    await switchToPersonal();

    expect(push).toHaveBeenCalledWith({ name: 'notes', params: { slug: 'restored-note' } });
  });

  it('lands on the section root when the destination workspace has no history (ATL-77)', async () => {
    routeState.name = 'task-detail';
    seed();

    await switchToPersonal();

    expect(push).toHaveBeenCalledWith({ name: 'tasks' });
  });

  it('purges the real current workspace cache before reloading the current route', async () => {
    const workspace = seed();
    const events: string[] = [];
    const deleteScope = vi.fn().mockImplementation(async () => {
      events.push('purge');
      return true;
    });
    configureResourceCacheForTest(
      new ResourceCache({
        store: {
          get: vi.fn(),
          putMany: vi.fn().mockResolvedValue(true),
          deleteMany: vi.fn().mockResolvedValue(true),
          deleteScope,
          clear: vi.fn().mockResolvedValue(true),
        },
      }),
    );
    setResourceCachePrincipal('user:019ef171-bbcf-7b90-9be6-5dbb382afd08');
    go.mockImplementation(() => {
      events.push('reload');
    });
    const wrapper = mount(AppRail);

    await wrapper.get('[aria-label="Account"]').trigger('click');
    await wrapper.get('[data-action="hard-refresh"]').trigger('click');
    expect(events).toEqual([]);
    await wrapper.findComponent(ConfirmDialog).vm.$emit('confirm');
    await flushPromises();

    expect(events).toEqual(['purge', 'reload']);
    expect(deleteScope).toHaveBeenCalledWith({
      principal: 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08',
      workspaceId: workspace.workspaces[0]?.id,
    });
  });

  it('keeps the confirmation open when the real workspace cache purge fails', async () => {
    seed();
    configureResourceCacheForTest(
      new ResourceCache({
        store: {
          get: vi.fn(),
          putMany: vi.fn().mockResolvedValue(true),
          deleteMany: vi.fn().mockResolvedValue(true),
          deleteScope: vi.fn().mockResolvedValue(false),
          clear: vi.fn().mockResolvedValue(true),
        },
      }),
    );
    setResourceCachePrincipal('user:019ef171-bbcf-7b90-9be6-5dbb382afd08');
    const wrapper = mount(AppRail);

    await wrapper.get('[aria-label="Account"]').trigger('click');
    await wrapper.get('[data-action="hard-refresh"]').trigger('click');
    await wrapper.findComponent(ConfirmDialog).vm.$emit('confirm');
    await flushPromises();

    expect(go).not.toHaveBeenCalled();
    expect(wrapper.findComponent(ConfirmDialog).props('open')).toBe(true);
    expect(useUiStore().banner).toMatchObject({
      message: 'Could not refresh cached data. Try again.',
      type: 'error',
    });
  });
});
