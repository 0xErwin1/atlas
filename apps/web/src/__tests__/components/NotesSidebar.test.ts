import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { LiveUpdateHandlers } from '@/composables/useLiveUpdates';

vi.mock('vue-router', () => ({
  useRoute: () => ({ params: {} }),
  useRouter: () => ({ push: vi.fn() }),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET: vi.fn().mockResolvedValue({ data: { items: [] }, error: undefined }),
  },
}));

const { useLiveUpdates } = vi.hoisted(() => ({ useLiveUpdates: vi.fn() }));
vi.mock('@/composables/useLiveUpdates', () => ({ useLiveUpdates }));

import Dropdown from '@/components/ui/Dropdown.vue';
import { useDocumentsStore } from '@/stores/documents';
import { useFoldersStore } from '@/stores/folders';
import { useWorkspaceStore } from '@/stores/workspace';
import NotesSidebar from '@/views/NotesSidebar.vue';

function setup() {
  const workspace = useWorkspaceStore();
  workspace.setActiveWorkspace('atlas');
  workspace.projects = [
    { slug: 'sandbox', name: 'Sandbox', task_prefix: 'SBX', workspace_id: 'w1', visibility: 'workspace' },
    { slug: 'roadmap', name: 'Roadmap', task_prefix: 'RD', workspace_id: 'w1', visibility: 'workspace' },
  ];
  const docs = useDocumentsStore();
  const folders = useFoldersStore();
  const loadSummaries = vi.spyOn(docs, 'loadSummaries').mockResolvedValue();
  vi.spyOn(folders, 'load').mockResolvedValue();
  return { loadSummaries };
}

function capturedLiveHandlers(): LiveUpdateHandlers {
  const handlers = useLiveUpdates.mock.calls.at(-1)?.[1] as LiveUpdateHandlers | undefined;
  if (handlers === undefined) throw new Error('Expected NotesSidebar to register live update handlers');
  return handlers;
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

  it('silently refreshes document summaries only for document events and reloads the tree on resync', async () => {
    const { loadSummaries } = setup();
    const wrapper = mount(NotesSidebar);
    await wrapper.vm.$nextTick();

    const folders = useFoldersStore();
    const loadFolders = vi.spyOn(folders, 'load').mockResolvedValue();
    loadSummaries.mockClear();
    loadFolders.mockClear();

    const handlers = capturedLiveHandlers();
    handlers.onEvent({ type: 'document.updated', data: {}, envelope: {} as never });
    handlers.onEvent({ type: 'task.updated', data: {}, envelope: {} as never });

    expect(loadSummaries).toHaveBeenCalledTimes(1);
    expect(loadSummaries).toHaveBeenCalledWith('atlas', 'sandbox', { silent: true });

    handlers.onResync?.();
    await wrapper.vm.$nextTick();

    expect(loadFolders).toHaveBeenCalledWith('atlas', 'sandbox');
    expect(loadSummaries).toHaveBeenLastCalledWith('atlas', 'sandbox');
  });
});
