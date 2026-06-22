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
  },
}));

import Dropdown from '@/components/ui/Dropdown.vue';
import { useDocumentsStore } from '@/stores/documents';
import { useFoldersStore } from '@/stores/folders';
import { useWorkspaceStore } from '@/stores/workspace';
import NotesSidebar from '@/views/NotesSidebar.vue';

function setup() {
  const workspace = useWorkspaceStore();
  workspace.setActiveWorkspace('atlas');
  workspace.projects = [
    { slug: 'sandbox', name: 'Sandbox', task_prefix: 'SBX', workspace_id: 'w1' },
    { slug: 'roadmap', name: 'Roadmap', task_prefix: 'RD', workspace_id: 'w1' },
  ];
  const docs = useDocumentsStore();
  const folders = useFoldersStore();
  const loadSummaries = vi.spyOn(docs, 'loadSummaries').mockResolvedValue();
  vi.spyOn(folders, 'load').mockResolvedValue();
  return { loadSummaries };
}

describe('NotesSidebar project selector', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    try {
      localStorage.clear();
    } catch {
      // jsdom always provides localStorage; ignore if absent
    }
  });

  it('lists every project as a switch option', async () => {
    setup();
    const wrapper = mount(NotesSidebar);
    await wrapper.vm.$nextTick();

    const dd = wrapper.findComponent(Dropdown);
    expect(dd.exists()).toBe(true);
    const opts = dd.props('options') as Array<{ value: string; label: string }>;
    expect(opts.map((o) => o.value)).toEqual(['sandbox', 'roadmap']);
  });

  it('loads the chosen project documents when switched', async () => {
    const { loadSummaries } = setup();
    const wrapper = mount(NotesSidebar);
    await wrapper.vm.$nextTick();

    loadSummaries.mockClear();
    wrapper.findComponent(Dropdown).vm.$emit('change', 'roadmap');
    await wrapper.vm.$nextTick();

    expect(loadSummaries).toHaveBeenCalledWith('atlas', 'roadmap');
  });
});
