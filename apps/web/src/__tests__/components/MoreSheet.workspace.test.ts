import { mount } from '@vue/test-utils';
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

import MoreSheet from '@/components/shell/MoreSheet.vue';
import { useWorkspaceStore } from '@/stores/workspace';

function seedWorkspaces() {
  const workspace = useWorkspaceStore();
  workspace.setActiveWorkspace('atlas');
  workspace.workspaces = [
    { id: '1', name: 'Atlas HQ', slug: 'atlas', created_at: 'x', updated_at: 'x' },
    { id: '2', name: '', slug: 'personal', created_at: 'x', updated_at: 'x' },
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
});
