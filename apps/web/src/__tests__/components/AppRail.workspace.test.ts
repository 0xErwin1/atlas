import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { routeState, push } = vi.hoisted(() => ({
  routeState: { name: 'notes' as string },
  push: vi.fn(),
}));

vi.mock('vue-router', () => ({
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

  it('keeps the user in Tasks after switching from a task route (ATL-49)', async () => {
    routeState.name = 'task-detail';
    seed();

    await switchToPersonal();

    expect(push).toHaveBeenCalledWith({ name: 'tasks' });
  });

  it('lands on Notes after switching from a note route (ATL-49)', async () => {
    routeState.name = 'notes';
    seed();

    await switchToPersonal();

    expect(push).toHaveBeenCalledWith({ name: 'notes' });
  });
});
