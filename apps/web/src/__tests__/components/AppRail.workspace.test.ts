import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { go, routeState, push, replace } = vi.hoisted(() => ({
  go: vi.fn(),
  routeState: { name: 'notes' as string, fullPath: '/n' },
  push: vi.fn(),
  replace: vi.fn(),
}));

vi.mock('vue-router', () => ({
  useRoute: () => routeState,
  useRouter: () => ({ go, push, replace }),
}));

import { configureResourceCacheForTest, setResourceCachePrincipal } from '@/cache/cacheRuntime';
import { ResourceCache } from '@/cache/resourceCache';
import AppRail from '@/components/shell/AppRail.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

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
  ];
  return workspace;
}

describe('AppRail hard refresh', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
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

  it('runs only one purge and reload for overlapping hard-refresh confirmations', async () => {
    seed();
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
    const wrapper = mount(AppRail);

    await wrapper.get('[aria-label="Account"]').trigger('click');
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
