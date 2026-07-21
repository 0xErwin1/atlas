import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { routeState, push } = vi.hoisted(() => ({
  routeState: { name: 'notes' as string, fullPath: '/n' },
  push: vi.fn(),
}));

vi.mock('vue-router', () => ({
  isNavigationFailure: vi.fn(() => false),
  NavigationFailureType: { redirected: 2, aborted: 4, cancelled: 8, duplicated: 16 },
  useRoute: () => routeState,
  useRouter: () => ({ go: vi.fn(), push, replace: vi.fn() }),
}));

import { configureResourceCacheForTest } from '@/cache/cacheRuntime';
import AppRail from '@/components/shell/AppRail.vue';
import { useWorkspaceStore } from '@/stores/workspace';

function seed() {
  const workspace = useWorkspaceStore();
  workspace.setActiveWorkspace('atlas');
  workspace.workspaces = [{ id: 'w1', name: 'Atlas', slug: 'atlas', created_at: 'x', updated_at: 'x' }];
}

describe('AppRail unified navigation', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    routeState.name = 'notes';
    configureResourceCacheForTest({
      allow: vi.fn(),
      block: vi.fn(),
      clear: vi.fn().mockResolvedValue(undefined),
      purge: vi.fn().mockResolvedValue(undefined),
      purgeTags: vi.fn().mockResolvedValue(undefined),
      purgeWorkspace: vi.fn().mockImplementation(async () => undefined),
    });
    seed();
  });

  it('labels the unified entry as Acta and keeps search out of the rail', () => {
    const wrapper = mount(AppRail);

    expect(wrapper.find('[aria-label="Acta"]').exists()).toBe(true);
    expect(wrapper.find('[aria-label="Search"]').exists()).toBe(false);
    expect(wrapper.find('[aria-label="Notes"]').exists()).toBe(false);
    expect(wrapper.find('[aria-label="Tasks"]').exists()).toBe(false);
  });

  it('navigates the unified entry to the notes route', async () => {
    const wrapper = mount(AppRail);

    await wrapper.get('[aria-label="Acta"]').trigger('click');

    expect(push).toHaveBeenCalledWith({ name: 'notes' });
  });

  it.each([
    'notes',
    'tasks',
    'task-view',
    'task-detail',
  ])('marks the unified entry active on the %s route', (routeName) => {
    routeState.name = routeName;
    const wrapper = mount(AppRail);

    expect(wrapper.get('[aria-label="Acta"]').attributes('aria-current')).toBe('page');
  });
});
