import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ZodType } from 'zod';
import { configureResourceCacheForTest, setResourceCachePrincipal } from '@/cache/cacheRuntime';
import {
  type CacheEnvelope,
  createCacheEnvelope,
  ResourceCache,
  type ResourceCacheStore,
} from '@/cache/resourceCache';
import type { LiveUpdateHandlers } from '@/composables/useLiveUpdates';

const { GET, PATCH } = vi.hoisted(() => ({ GET: vi.fn(), PATCH: vi.fn() }));

vi.mock('vue-router', () => ({
  useRoute: () => ({ params: {} }),
  useRouter: () => ({ push: vi.fn() }),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET,
    PATCH,
  },
}));

const { useLiveUpdates } = vi.hoisted(() => ({ useLiveUpdates: vi.fn() }));
vi.mock('@/composables/useLiveUpdates', () => ({ useLiveUpdates }));

import NotesTree from '@/components/notas/NotesTree.vue';
import Dropdown from '@/components/ui/Dropdown.vue';
import { useDocumentsStore } from '@/stores/documents';
import { useFoldersStore } from '@/stores/folders';
import { useWorkspaceStore } from '@/stores/workspace';
import NotesSidebar from '@/views/NotesSidebar.vue';

const PRINCIPAL = 'user:018f4abc-1234-7abc-8def-0123456789ab';
const WORKSPACE_ID = '018f4abc-1234-7abc-8def-0123456789ac';

type Catalog = {
  folders: Array<{
    id: string;
    name: string;
    parent_folder_id: string | null;
    project_id: string | null;
    workspace_id: string;
    created_at: string;
    updated_at: string;
  }>;
  summaries: Array<{
    id: string;
    slug: string;
    title: string;
    folder_id: string | null;
    head_seq: number;
    updated_at: string;
  }>;
};

class MemoryCacheStore implements ResourceCacheStore {
  readonly entries = new Map<string, CacheEnvelope<unknown>>();

  async get<T>(key: string, _payloadSchema: ZodType<T>): Promise<CacheEnvelope<T> | null> {
    return (this.entries.get(key) as CacheEnvelope<T> | undefined) ?? null;
  }

  async putMany(entries: readonly CacheEnvelope<unknown>[]): Promise<boolean> {
    for (const entry of entries) this.entries.set(entry.key, entry);
    return true;
  }

  async deleteMany(keys: readonly string[]): Promise<boolean> {
    for (const key of keys) this.entries.delete(key);
    return true;
  }

  async clear(): Promise<boolean> {
    this.entries.clear();
    return true;
  }
}

function catalog(folderName: string, documentTitle: string): Catalog {
  return {
    folders: [
      {
        id: `${folderName}-folder`,
        name: folderName,
        parent_folder_id: null,
        project_id: 'project-id',
        workspace_id: WORKSPACE_ID,
        created_at: '2026-01-01T00:00:00Z',
        updated_at: '2026-01-01T00:00:00Z',
      },
    ],
    summaries: [
      {
        id: `${documentTitle}-document`,
        slug: documentTitle.toLowerCase().replaceAll(' ', '-'),
        title: documentTitle,
        folder_id: null,
        head_seq: 1,
        updated_at: '2026-01-01T00:00:00Z',
      },
    ],
  };
}

function seedCatalog(store: MemoryCacheStore, projectSlug: string, payload: Catalog): void {
  const key = `v1|p=${PRINCIPAL}|w=${WORKSPACE_ID}|k=note-tree|r=${projectSlug}|q={}`;
  const now = Date.now();
  store.entries.set(
    key,
    createCacheEnvelope({
      key,
      payloadVersion: 1,
      storedAt: now,
      validatedAt: now,
      lastAccessedAt: now,
      retentionExpiresAt: now + 60_000,
      bytes: JSON.stringify(payload).length,
      stale: false,
      tags: [`project:${projectSlug}`],
      payload,
    }),
  );
}

function configureCatalogRuntime(store: MemoryCacheStore): void {
  const cache = new ResourceCache({ store });
  cache.allow();
  configureResourceCacheForTest(cache);
  setResourceCachePrincipal(PRINCIPAL);
}

function setupCatalogWorkspace() {
  const workspace = useWorkspaceStore();
  workspace.setActiveWorkspace('atlas');
  workspace.projects = [
    {
      slug: 'sandbox',
      name: 'Sandbox',
      task_prefix: 'SBX',
      workspace_id: WORKSPACE_ID,
      visibility: 'workspace',
    },
  ];
  vi.spyOn(workspace, 'workspaceIdForSlug').mockReturnValue(WORKSPACE_ID);
  return workspace;
}

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
    GET.mockResolvedValue({ data: { items: [] }, error: undefined });
    PATCH.mockResolvedValue({ data: {}, error: undefined });
    setResourceCachePrincipal(undefined);
    try {
      localStorage.clear();
    } catch {
      // jsdom always provides localStorage; ignore if absent
    }
  });

  it.each([403, 404])('retracts cached catalog state before showing a known denial (%i)', async (status) => {
    const store = new MemoryCacheStore();
    seedCatalog(store, 'sandbox', catalog('Cached folder', 'Cached document'));
    configureCatalogRuntime(store);
    GET.mockResolvedValue({ error: Object.assign(new Error('Denied'), { status }) });
    setupCatalogWorkspace();

    const wrapper = mount(NotesSidebar);
    await flushPromises();

    expect(useFoldersStore().foldersByProject).toEqual({ sandbox: [] });
    expect(useDocumentsStore().summariesByProject).toEqual({ sandbox: [] });
    expect(wrapper.findComponent(NotesTree).exists()).toBe(false);
    expect(wrapper.text()).toContain('Couldn’t load notes');
    wrapper.unmount();
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

  it('refreshes the catalog atomically for document events and resync', async () => {
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
    expect(loadSummaries).toHaveBeenCalledWith('atlas', 'sandbox');

    handlers.onResync?.();
    await wrapper.vm.$nextTick();

    expect(loadFolders).toHaveBeenCalledWith('atlas', 'sandbox');
    expect(loadSummaries).toHaveBeenLastCalledWith('atlas', 'sandbox');
  });

  it('hydrates cached folders and summaries together before a pending network refresh, then publishes both refresh results', async () => {
    const store = new MemoryCacheStore();
    const cached = catalog('Cached folder', 'Cached document');
    const refreshed = catalog('Fresh folder', 'Fresh document');
    seedCatalog(store, 'sandbox', cached);
    configureCatalogRuntime(store);

    let resolveFolders: (value: { data: { items: Catalog['folders']; has_more: boolean } }) => void =
      () => {};
    let resolveSummaries: (value: { data: { items: Catalog['summaries']; has_more: boolean } }) => void =
      () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveFolders = resolve;
      }),
    ).mockReturnValueOnce(
      new Promise((resolve) => {
        resolveSummaries = resolve;
      }),
    );

    setupCatalogWorkspace();
    const wrapper = mount(NotesSidebar);
    await flushPromises();

    expect(
      useFoldersStore()
        .foldersFor('sandbox')
        .map((folder) => folder.name),
    ).toEqual(['Cached folder']);
    expect(
      useDocumentsStore()
        .summariesFor('sandbox')
        .map((summary) => summary.title),
    ).toEqual(['Cached document']);
    expect(wrapper.text()).toContain('Cached folder');
    expect(wrapper.text()).toContain('Cached document');

    resolveFolders({ data: { items: refreshed.folders, has_more: false } });
    resolveSummaries({ data: { items: refreshed.summaries, has_more: false } });
    await flushPromises();

    expect(
      useFoldersStore()
        .foldersFor('sandbox')
        .map((folder) => folder.name),
    ).toEqual(['Fresh folder']);
    expect(
      useDocumentsStore()
        .summariesFor('sandbox')
        .map((summary) => summary.title),
    ).toEqual(['Fresh document']);
    expect(wrapper.text()).toContain('Fresh folder');
    expect(wrapper.text()).toContain('Fresh document');
  });

  it('does not let a stale cached composite overwrite the store refresh after a successful rename', async () => {
    const store = new MemoryCacheStore();
    const stale = catalog('Existing folder', 'Old title');
    const renamed = catalog('Existing folder', 'Renamed title');
    seedCatalog(store, 'sandbox', stale);
    configureCatalogRuntime(store);

    GET.mockResolvedValueOnce({ data: { items: stale.folders, has_more: false } })
      .mockResolvedValueOnce({ data: { items: stale.summaries, has_more: false } })
      .mockResolvedValueOnce({ data: { items: renamed.summaries, has_more: false } });
    setupCatalogWorkspace();
    const wrapper = mount(NotesSidebar);
    await flushPromises();

    wrapper.findComponent(NotesTree).vm.$emit('rename-doc', 'old-title', 'Renamed title');
    await flushPromises();
    await wrapper.vm.$nextTick();

    expect(
      useDocumentsStore()
        .summariesFor('sandbox')
        .map((summary) => summary.title),
    ).toEqual(['Renamed title']);
    GET.mockReturnValueOnce(new Promise(() => {})).mockReturnValueOnce(new Promise(() => {}));
    capturedLiveHandlers().onResync?.();
    await flushPromises();
    await wrapper.vm.$nextTick();

    expect(
      useDocumentsStore()
        .summariesFor('sandbox')
        .map((summary) => summary.title),
    ).toEqual(['Renamed title']);
  });

  it('synchronously clears the prior principal catalog before loading under the next principal', async () => {
    const priorPrincipal = PRINCIPAL;
    const nextPrincipal = 'user:018f4abc-1234-7abc-8def-0123456789ad';
    const store = new MemoryCacheStore();
    seedCatalog(store, 'sandbox', catalog('Prior folder', 'Prior document'));
    configureCatalogRuntime(store);
    GET.mockReturnValue(new Promise(() => {}));
    setupCatalogWorkspace();

    const wrapper = mount(NotesSidebar);
    await flushPromises();
    expect(wrapper.text()).toContain('Prior document');

    setResourceCachePrincipal(nextPrincipal);
    await wrapper.vm.$nextTick();

    expect(wrapper.text()).not.toContain('Prior folder');
    expect(wrapper.text()).not.toContain('Prior document');
    expect(wrapper.text()).toContain('Loading notes…');
    expect(priorPrincipal).not.toBe(nextPrincipal);
  });

  it('clears active and non-active project buckets before a new principal catalog can render', async () => {
    const store = new MemoryCacheStore();
    seedCatalog(store, 'sandbox', catalog('Prior folder', 'Prior document'));
    configureCatalogRuntime(store);
    GET.mockReturnValue(new Promise(() => {}));
    setupCatalogWorkspace();

    const wrapper = mount(NotesSidebar);
    await flushPromises();
    useFoldersStore().publishForProject('roadmap', catalog('Other folder', 'Other document').folders);
    useDocumentsStore().publishSummariesForProject(
      'roadmap',
      catalog('Other folder', 'Other document').summaries,
    );

    setResourceCachePrincipal('user:018f4abc-1234-7abc-8def-0123456789ad');
    await wrapper.vm.$nextTick();

    expect(useFoldersStore().foldersByProject).toEqual({});
    expect(useDocumentsStore().summariesByProject).toEqual({});
    expect(wrapper.findComponent(NotesTree).exists()).toBe(false);
    expect(wrapper.text()).toContain('Loading notes…');
  });

  it('uses the empty loader and error lifecycle when catalog caching has no usable key', async () => {
    vi.restoreAllMocks();
    GET.mockReset();
    setResourceCachePrincipal(undefined);
    setupCatalogWorkspace();
    let resolveFolders: (value: { error: { hint: string } }) => void = () => {};
    let resolveSummaries: (value: { error: { hint: string } }) => void = () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveFolders = resolve;
      }),
    ).mockReturnValueOnce(
      new Promise((resolve) => {
        resolveSummaries = resolve;
      }),
    );

    const wrapper = mount(NotesSidebar);
    await wrapper.vm.$nextTick();
    expect(wrapper.text()).toContain('Loading notes…');

    resolveFolders({ error: { hint: 'folders unavailable' } });
    resolveSummaries({ error: { hint: 'documents unavailable' } });
    await flushPromises();
    await wrapper.vm.$nextTick();

    expect(useFoldersStore().error).toBe('folders unavailable');
    expect(useDocumentsStore().error).toBe('documents unavailable');
    expect(wrapper.text()).toContain('Couldn’t load notes');
    expect(wrapper.text()).not.toContain('Loading notes…');
  });

  it('waits for the switched workspace project list before loading a catalog', async () => {
    const workspace = setupCatalogWorkspace();
    const wrapper = mount(NotesSidebar);
    await flushPromises();
    GET.mockClear();

    workspace.switchWorkspace('other-workspace');
    await wrapper.vm.$nextTick();

    expect(GET).not.toHaveBeenCalledWith(
      '/api/workspaces/{ws}/projects/{project_slug}/folders',
      expect.objectContaining({
        params: expect.objectContaining({ path: { ws: 'other-workspace', project_slug: 'sandbox' } }),
      }),
    );
    expect(GET).not.toHaveBeenCalledWith(
      '/api/workspaces/{ws}/projects/{project_slug}/documents',
      expect.objectContaining({
        params: expect.objectContaining({ path: { ws: 'other-workspace', project_slug: 'sandbox' } }),
      }),
    );

    wrapper.unmount();
  });
});
