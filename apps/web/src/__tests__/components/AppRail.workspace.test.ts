import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { isNavigationFailure, routeState, push } = vi.hoisted(() => ({
  isNavigationFailure: vi.fn((_result: unknown) => false),
  routeState: { name: 'notes' as string },
  push: vi.fn(),
}));

vi.mock('vue-router', () => ({
  isNavigationFailure,
  useRoute: () => routeState,
  useRouter: () => ({ push }),
}));

import AppRail from '@/components/shell/AppRail.vue';
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
    { id: '1', name: 'Atlas', slug: 'atlas', created_at: 'x', updated_at: 'x' },
    { id: '2', name: 'Personal', slug: 'personal', created_at: 'x', updated_at: 'x' },
  ];
  return workspace;
}

describe('AppRail workspace switcher', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    isNavigationFailure.mockReturnValue(false);
    routeState.name = 'notes';
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
    const failure = { type: 'aborted' };
    push.mockResolvedValueOnce(failure);
    isNavigationFailure.mockImplementationOnce((result) => result === failure);
    const workspace = seed();

    await switchToPersonal();

    expect(workspace.activeWorkspaceSlug).toBe('atlas');
  });

  it('restores the old workspace when navigation rejects', async () => {
    push.mockRejectedValueOnce(new Error('navigation rejected'));
    const workspace = seed();

    await switchToPersonal();
    await flushPromises();

    expect(workspace.activeWorkspaceSlug).toBe('atlas');
  });
});
