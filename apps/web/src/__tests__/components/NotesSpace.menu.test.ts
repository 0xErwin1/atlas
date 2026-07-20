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

const { useLiveUpdates } = vi.hoisted(() => ({ useLiveUpdates: vi.fn() }));
vi.mock('@/composables/useLiveUpdates', () => ({ useLiveUpdates }));

import NotesSpace from '@/components/notas/NotesSpace.vue';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import Row from '@/components/ui/Row.vue';
import type { ProjectSummary } from '@/stores/workspace';
import { useWorkspaceStore } from '@/stores/workspace';

const ROADMAP: ProjectSummary = {
  slug: 'roadmap',
  name: 'Roadmap',
  task_prefix: 'RD',
  workspace_id: 'w1',
  visibility: 'workspace',
};

function mountSpace() {
  const workspace = useWorkspaceStore();
  workspace.setActiveWorkspace('atlas');
  return mount(NotesSpace, {
    props: { project: ROADMAP, activeSlug: null, activeBoardId: null },
  });
}

async function headerMenuLabels(wrapper: ReturnType<typeof mount>): Promise<string[]> {
  const header = wrapper.findAllComponents(Row).find((r) => r.props('label') === 'Roadmap');
  if (header === undefined) throw new Error('space header row not found');

  header.vm.$emit('menu', { clientX: 10, clientY: 10, preventDefault() {} });
  await wrapper.vm.$nextTick();

  const menu = wrapper.findAllComponents(ContextMenu).at(-1);
  if (menu === undefined) throw new Error('space header context menu not found');

  return (menu.props('items') as Array<{ label?: string }>)
    .map((item) => item.label)
    .filter((label): label is string => typeof label === 'string');
}

describe('NotesSpace header context menu', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('offers New page, New board and New folder on a space', async () => {
    const wrapper = mountSpace();
    await wrapper.vm.$nextTick();

    const labels = await headerMenuLabels(wrapper);

    expect(labels).toContain('New page');
    expect(labels).toContain('New board');
    expect(labels).toContain('New folder');
  });
});
