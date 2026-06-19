import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('vue-router', () => ({
  useRoute: () => ({ name: 'notes' }),
  useRouter: () => ({ push: vi.fn() }),
}));

import AppRail from '@/components/shell/AppRail.vue';
import { useWorkspaceStore } from '@/stores/workspace';

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
    const wrapper = mount(AppRail);

    await wrapper.get('[aria-label="Switch workspace"]').trigger('click');
    const items = wrapper.findAll('.atl-account-item');
    const personal = items.find((b) => b.text().includes('Personal'));
    if (personal === undefined) throw new Error('Personal workspace item not rendered');
    await personal.trigger('click');

    expect(spy).toHaveBeenCalledWith('personal');
  });
});
