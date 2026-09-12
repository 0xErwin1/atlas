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
import { useDiscoveryStore } from '@/stores/discovery';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

function seed() {
  const workspace = useWorkspaceStore();
  workspace.setActiveWorkspace('atlas');
  workspace.workspaces = [{ id: 'w1', name: 'Atlas', slug: 'atlas', created_at: 'x', updated_at: 'x' }];

  // The rail composes against `discovery` (E11-S8 PR4) — seed it so the
  // pre-existing Acta rail entry keeps appearing for these route/navigation
  // assertions, which are unrelated to the composer itself (that logic is
  // unit-tested in `navigationManifest.test.ts`/`discovery.test.ts`).
  const discovery = useDiscoveryStore();
  discovery.metaComponents = [
    { stable_id: 'acta', kind: 'product', contract_version: 1, navigation_providers: ['acta.workspace'] },
  ];
  discovery.discover = {
    admin: false,
    truncated: false,
    components: [{ component: 'acta', scopes: ['acta::workspace::w1'] }],
  };
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

  // Search is a route inside Acta, so the rail has to keep saying which product
  // you are in while you are there — it is the only way back to the space tree.
  it.each([
    'notes',
    'tasks',
    'task-view',
    'task-detail',
    'search',
  ])('marks the Acta entry current on the %s route', (name) => {
    routeState.name = name;
    const wrapper = mount(AppRail);

    expect(wrapper.get('[aria-label="Acta"]').attributes('aria-current')).toBe('page');
  });

  it('leaves the rail with no current product outside Acta', () => {
    routeState.name = 'settings';
    const wrapper = mount(AppRail);

    expect(wrapper.get('[aria-label="Acta"]').attributes('aria-current')).toBeUndefined();
  });

  it('navigates the unified entry to the notes route', async () => {
    const wrapper = mount(AppRail);

    await wrapper.get('[aria-label="Acta"]').trigger('click');

    expect(push).toHaveBeenCalledWith({ name: 'notes' });
  });

  it('restores a persisted collapsed sidebar from the persistent rail control', async () => {
    const ui = useUiStore();
    ui.sidebarCollapsed = true;
    const wrapper = mount(AppRail);

    const toggle = wrapper.get('[aria-label="Expand sidebar"]');
    expect(toggle.attributes('title')).toBe('Expand sidebar');
    expect(toggle.attributes('disabled')).toBeUndefined();

    await toggle.trigger('click');

    expect(ui.sidebarCollapsed).toBe(false);
    expect(wrapper.get('[aria-label="Collapse sidebar"]').attributes('title')).toBe('Collapse sidebar');
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

describe('AppRail navigation error state', () => {
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
  });

  it('shows a retry action carrying the error message when discovery fails and composes nothing', async () => {
    const discovery = useDiscoveryStore();
    discovery.error = 'Failed to load discoverable navigation';
    const retry = vi.spyOn(discovery, 'retry').mockResolvedValue(undefined);
    const wrapper = mount(AppRail);

    const retryButton = wrapper.get('[aria-label="Retry loading navigation"]');
    expect(retryButton.attributes('title')).toBe('Failed to load discoverable navigation');

    await retryButton.trigger('click');

    expect(retry).toHaveBeenCalledOnce();
  });
});
