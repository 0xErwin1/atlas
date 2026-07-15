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
import MoreSheet from '@/components/shell/MoreSheet.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

function seedWorkspaces() {
  const workspace = useWorkspaceStore();
  workspace.setActiveWorkspace('atlas');
  workspace.workspaces = [
    {
      id: '019ef171-bbcf-7b90-9be6-5dbb382afd08',
      name: 'Atlas HQ',
      slug: 'atlas',
      created_at: 'x',
      updated_at: 'x',
    },
    {
      id: '019ef171-bbcf-7b90-9be6-5dbb382afd09',
      name: '',
      slug: 'personal',
      created_at: 'x',
      updated_at: 'x',
    },
  ];

  return workspace;
}

describe('MoreSheet workspace switcher', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    isNavigationFailure.mockReturnValue(false);
    routeState.name = 'notes';
    push.mockResolvedValue(undefined);
    localStorage.clear();
    configureResourceCacheForTest({
      allow: vi.fn(),
      block: vi.fn(),
      clear: vi.fn().mockResolvedValue(undefined),
      purge: vi.fn().mockResolvedValue(undefined),
      purgeTags: vi.fn().mockResolvedValue(undefined),
      purgeWorkspace: vi.fn().mockResolvedValue(undefined),
    });
  });

  it('lists workspaces by name with slug fallback and marks the active workspace', () => {
    seedWorkspaces();

    const wrapper = mount(MoreSheet, { props: { open: true } });
    const options = wrapper.findAll('[data-workspace-option]');

    expect(options.map((option) => option.text())).toEqual(['Atlas HQ', 'personal']);
    expect(options[0]?.attributes('aria-current')).toBe('true');
    expect(options[1]?.attributes('aria-current')).toBeUndefined();
  });

  it('switches workspace through the shared flow and closes the sheet', async () => {
    const workspace = seedWorkspaces();
    const switchWorkspace = vi.spyOn(workspace, 'switchWorkspace');
    const wrapper = mount(MoreSheet, { props: { open: true } });

    await wrapper.get('[data-workspace-option="personal"]').trigger('click');

    expect(push).toHaveBeenCalledWith({ name: 'notes' });
    expect(switchWorkspace).toHaveBeenCalledWith('personal');
    expect(wrapper.emitted('close')).toHaveLength(1);
  });

  it('purges the real current workspace cache before reloading the current route', async () => {
    const workspace = seedWorkspaces();
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
    const wrapper = mount(MoreSheet, { props: { open: true } });

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
    seedWorkspaces();
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
    const wrapper = mount(MoreSheet, { props: { open: true } });

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

  it('runs only one purge and reload for overlapping hard-refresh confirmations', async () => {
    seedWorkspaces();
    let resolveDelete: ((result: boolean) => void) | undefined;
    const deleteScope = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          resolveDelete = resolve;
        }),
    );
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
    const wrapper = mount(MoreSheet, { props: { open: true } });

    await wrapper.get('[data-action="hard-refresh"]').trigger('click');
    const dialog = wrapper.findComponent(ConfirmDialog);
    dialog.vm.$emit('confirm');
    dialog.vm.$emit('confirm');
    await vi.waitFor(() => expect(deleteScope).toHaveBeenCalledOnce());

    resolveDelete?.(true);
    await flushPromises();

    expect(go).toHaveBeenCalledOnce();
  });
});
