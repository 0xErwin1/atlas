import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('vue-router', () => ({
  useRoute: () => ({ params: {} }),
  useRouter: () => ({ push: vi.fn() }),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET: vi.fn().mockResolvedValue({ data: { items: [] }, error: undefined }),
    POST: vi.fn(),
    PATCH: vi.fn(),
    DELETE: vi.fn(),
  },
}));

import ContextMenu from '@/components/ui/ContextMenu.vue';
import Row from '@/components/ui/Row.vue';
import { useWorkspaceStore } from '@/stores/workspace';
import TasksSidebar from '@/views/TasksSidebar.vue';

function mountWithProject() {
  const workspace = useWorkspaceStore();
  workspace.setActiveWorkspace('atlas');
  workspace.projects = [
    { slug: 'roadmap', name: 'Roadmap', task_prefix: 'RD', workspace_id: 'w1', visibility: 'workspace' },
  ];
  return mount(TasksSidebar);
}

async function projectMenuLabels(wrapper: ReturnType<typeof mount>): Promise<string[]> {
  const rows = wrapper.findAllComponents(Row);
  const projectRow = rows.find((r) => r.props('label') === 'Roadmap');
  if (!projectRow) throw new Error('project row not found');
  projectRow.vm.$emit('menu', { clientX: 10, clientY: 10, preventDefault() {} });
  await wrapper.vm.$nextTick();
  const menu = wrapper.findComponent(ContextMenu);
  return (menu.props('items') as Array<{ label?: string }>)
    .map((i) => i.label)
    .filter((l): l is string => typeof l === 'string');
}

describe('TasksSidebar project context menu', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('offers New document, New folder, Rename and Delete on a project', async () => {
    const wrapper = mountWithProject();
    await wrapper.vm.$nextTick();

    const labels = await projectMenuLabels(wrapper);

    expect(labels).toContain('New document');
    expect(labels).toContain('New folder');
    expect(labels).toContain('Rename');
    expect(labels).toContain('Delete');
  });

  it('keeps the existing New board and New project actions', async () => {
    const wrapper = mountWithProject();
    await wrapper.vm.$nextTick();

    const labels = await projectMenuLabels(wrapper);

    expect(labels).toContain('New board');
    expect(labels).toContain('New project');
  });
});
