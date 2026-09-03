import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ZodType } from 'zod';
import {
  allowResourceCache,
  blockAndPurgeResourceCache,
  configureResourceCacheForTest,
  setResourceCachePrincipal,
} from '@/cache/cacheRuntime';
import {
  type CacheEnvelope,
  createCacheEnvelope,
  ResourceCache,
  type ResourceCacheStore,
} from '@/cache/resourceCache';
import type { LiveUpdateHandlers } from '@/composables/useLiveUpdates';
import { EVENT_TYPE } from '@/lib/eventTypes';

const { GET, PATCH, PUT } = vi.hoisted(() => ({ GET: vi.fn(), PATCH: vi.fn(), PUT: vi.fn() }));
const { routerCurrentRoute, routerPush } = vi.hoisted(() => ({
  routerCurrentRoute: { value: { name: 'notes', params: { slug: 'doc-1' } } },
  routerPush: vi.fn(),
}));

vi.mock('vue-router', () => ({
  useRoute: () => ({ params: {} }),
  useRouter: () => ({ currentRoute: routerCurrentRoute, push: routerPush }),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET,
    PATCH,
    PUT,
  },
}));

const { useLiveUpdates } = vi.hoisted(() => ({ useLiveUpdates: vi.fn() }));
vi.mock('@/composables/useLiveUpdates', () => ({ useLiveUpdates }));

import NotesSpace from '@/components/notas/NotesSpace.vue';
import NotesTree from '@/components/notas/NotesTree.vue';
import { useAuthStore } from '@/stores/auth';
import { useBoardsStore } from '@/stores/boards';
import { useDocumentsStore } from '@/stores/documents';
import { useFoldersStore } from '@/stores/folders';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useNotesTabsStore } from '@/stores/notesTabs';
import { useUiStateStore } from '@/stores/uiState';
import type { ProjectSummary } from '@/stores/workspace';
import { useWorkspaceStore } from '@/stores/workspace';

const SESSION_USER_ID = '018f4abc-1234-7abc-8def-0123456789ab';
const OTHER_USER_ID = '018f4abc-1234-7abc-8def-0123456789ad';
const PRINCIPAL = `user:${SESSION_USER_ID}`;
const WORKSPACE_ID = '018f4abc-1234-7abc-8def-0123456789ac';
const SANDBOX_PROJECT_ID = '018f4abc-1234-7abc-8def-0123456789ae';

const SANDBOX: ProjectSummary = {
  id: SANDBOX_PROJECT_ID,
  slug: 'sandbox',
  name: 'Sandbox',
  task_prefix: 'SBX',
  workspace_id: WORKSPACE_ID,
  visibility: 'workspace',
};

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

  async deleteScope(scope: {
    principal: string;
    workspaceId?: string;
    tagsAny?: readonly string[];
  }): Promise<boolean> {
    for (const [key, entry] of this.entries) {
      const inWorkspace =
        key.includes(`|p=${scope.principal}|`) &&
        (scope.workspaceId === undefined || key.includes(`|w=${scope.workspaceId}|`));
      const hasTag = scope.tagsAny === undefined || entry.tags.some((tag) => scope.tagsAny?.includes(tag));
      if (inWorkspace && hasTag) this.entries.delete(key);
    }
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

function setupWorkspace() {
  const workspace = useWorkspaceStore();
  workspace.setActiveWorkspace('atlas');
  workspace.projects = [SANDBOX];
  vi.spyOn(workspace, 'workspaceIdForSlug').mockReturnValue(WORKSPACE_ID);
  return workspace;
}

function mountSpace() {
  return mount(NotesSpace, { props: { project: SANDBOX, activeSlug: null, activeBoardId: null } });
}

function capturedLiveHandlers(): LiveUpdateHandlers {
  const handlers = useLiveUpdates.mock.calls.at(-1)?.[1] as LiveUpdateHandlers | undefined;
  if (handlers === undefined) throw new Error('Expected NotesSpace to register live update handlers');
  return handlers;
}

const RENAMED_DOCUMENT = {
  id: 'Old title-document',
  slug: 'old-title',
  title: 'Renamed live',
  folder_id: null,
  head_seq: 3,
  updated_at: '2026-01-03T00:00:00Z',
  project_id: SANDBOX_PROJECT_ID,
  workspace_id: WORKSPACE_ID,
  content: '',
  frontmatter: {},
  head_revision_id: 'revision-id',
  created_at: '2026-01-02T00:00:00Z',
};

async function mountWithListedDocument() {
  setupWorkspace();
  const docs = useDocumentsStore();
  const loadSummaries = vi.spyOn(docs, 'loadSummaries').mockResolvedValue();
  vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
  vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue(null);

  const wrapper = mountSpace();
  await flushPromises();

  docs.publishSummariesForProject('sandbox', catalog('Folder', 'Old title').summaries);
  GET.mockClear();
  loadSummaries.mockClear();
  GET.mockResolvedValue({ error: undefined, data: RENAMED_DOCUMENT });

  return { wrapper, docs, loadSummaries };
}

function signIn(): void {
  useAuthStore().user = {
    id: SESSION_USER_ID,
    principal_type: 'user',
    username: 'bob',
    is_root: false,
    is_system_admin: false,
  };
}

function documentUpdatedFrame(actorId: string, documentId: string | null) {
  return {
    type: EVENT_TYPE.DOCUMENT_UPDATED,
    data: documentId === null ? {} : { document_id: documentId, revision_id: 'revision-id', seq: 3 },
    envelope: { project_id: SANDBOX_PROJECT_ID, actor: { type: 'user', id: actorId } } as never,
  };
}

describe('NotesSpace catalog', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    GET.mockResolvedValue({ data: { items: [] }, error: undefined });
    PATCH.mockResolvedValue({ data: {}, error: undefined });
    PUT.mockResolvedValue({ data: {}, error: undefined });
    setResourceCachePrincipal(undefined);
    try {
      localStorage.clear();
    } catch {
      // jsdom always provides localStorage; ignore if absent
    }
  });

  it('keeps the project row at the shared root padding', () => {
    const wrapper = mountSpace();

    expect(wrapper.get('.atl-row').attributes('style')).toContain('padding-left: 8px');
    wrapper.unmount();
  });

  it('restores project expansion by stable project id after remount', async () => {
    setupWorkspace();
    const uiState = useUiStateStore();
    uiState.setProjectCollapsed(WORKSPACE_ID, SANDBOX_PROJECT_ID, true);

    const collapsed = mountSpace();
    await flushPromises();
    expect(collapsed.findComponent(NotesTree).exists()).toBe(false);

    await collapsed.get('.atl-row').trigger('click');
    await flushPromises();
    expect(uiState.isProjectCollapsed(WORKSPACE_ID, SANDBOX_PROJECT_ID)).toBe(false);
    expect(collapsed.findComponent(NotesTree).exists()).toBe(true);
    collapsed.unmount();

    const remounted = mountSpace();
    await flushPromises();
    expect(remounted.findComponent(NotesTree).exists()).toBe(true);
    remounted.unmount();
  });

  it('does not load an old same-slug project against the next workspace during a switch', async () => {
    const workspace = setupWorkspace();
    const oldGeneral = { ...SANDBOX, slug: 'general', workspace_id: 'old-workspace-id' };
    workspace.projects = [oldGeneral];
    vi.spyOn(workspace, 'workspaceIdForSlug').mockReturnValue(WORKSPACE_ID);

    const wrapper = mount(NotesSpace, {
      props: { project: oldGeneral, activeSlug: null, activeBoardId: null },
    });
    await flushPromises();
    GET.mockClear();

    workspace.switchWorkspace('new-workspace');
    await flushPromises();

    expect(GET).not.toHaveBeenCalledWith(
      '/api/v2/acta/workspaces/{ws}/projects/{project_slug}/folders',
      expect.objectContaining({
        params: expect.objectContaining({ path: { ws: 'new-workspace', project_slug: 'general' } }),
      }),
    );
    wrapper.unmount();
  });

  it.each([403, 404])('retracts cached catalog state before showing a known denial (%i)', async (status) => {
    const store = new MemoryCacheStore();
    seedCatalog(store, 'sandbox', catalog('Cached folder', 'Cached document'));
    configureCatalogRuntime(store);
    GET.mockResolvedValue({ error: Object.assign(new Error('Denied'), { status }) });
    setupWorkspace();

    const wrapper = mountSpace();
    await flushPromises();

    expect(useFoldersStore().foldersByProject).toEqual({ sandbox: [] });
    expect(useDocumentsStore().summariesByProject).toEqual({ sandbox: [] });
    expect(wrapper.findComponent(NotesTree).exists()).toBe(false);
    expect(wrapper.text()).toContain('Couldn’t load notes');
    wrapper.unmount();
  });

  it('incrementally fetches and upserts a created document without reloading the catalog', async () => {
    setupWorkspace();
    const docs = useDocumentsStore();
    const loadSummaries = vi.spyOn(docs, 'loadSummaries').mockResolvedValue();
    const loadFolders = vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    const loadBoards = vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue(null);
    const wrapper = mountSpace();
    await flushPromises();
    GET.mockClear();
    loadSummaries.mockClear();
    loadFolders.mockClear();
    loadBoards.mockClear();
    GET.mockResolvedValueOnce({
      data: {
        id: 'new-document-id',
        slug: 'new-document',
        title: 'New document',
        folder_id: null,
        head_seq: 2,
        updated_at: '2026-01-02T00:00:00Z',
        project_id: SANDBOX_PROJECT_ID,
        workspace_id: WORKSPACE_ID,
        content: '',
        frontmatter: {},
        head_revision_id: 'revision-id',
        created_at: '2026-01-02T00:00:00Z',
      },
    });

    capturedLiveHandlers().onEvent({
      type: EVENT_TYPE.DOCUMENT_CREATED,
      data: { document_id: 'new-document-id', slug: 'new-document', title: 'New document' },
      envelope: { project_id: SANDBOX_PROJECT_ID } as never,
    });
    await flushPromises();

    expect(GET).toHaveBeenCalledOnce();
    expect(GET).toHaveBeenCalledWith('/api/v2/acta/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws: 'atlas', slug: 'new-document' } },
    });
    expect(docs.summariesFor('sandbox').map((item) => item.id)).toContain('new-document-id');
    expect(loadFolders).not.toHaveBeenCalled();
    expect(loadSummaries).not.toHaveBeenCalled();
    expect(loadBoards).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('falls back to the catalog reload when a relevant created payload is incomplete', async () => {
    setupWorkspace();
    const loadSummaries = vi.spyOn(useDocumentsStore(), 'loadSummaries').mockResolvedValue();
    vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue(null);
    const wrapper = mountSpace();
    await flushPromises();
    GET.mockClear();
    loadSummaries.mockClear();

    vi.useFakeTimers();
    capturedLiveHandlers().onEvent({
      type: EVENT_TYPE.DOCUMENT_CREATED,
      data: { document_id: 'new-document-id', slug: 'new-document' },
      envelope: { project_id: SANDBOX_PROJECT_ID } as never,
    });
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(GET).not.toHaveBeenCalled();
    expect(loadSummaries).toHaveBeenCalledWith('atlas', 'sandbox');
    wrapper.unmount();
  });

  it('falls back to the catalog reload when the targeted document fetch fails', async () => {
    setupWorkspace();
    const docs = useDocumentsStore();
    const loadSummaries = vi.spyOn(docs, 'loadSummaries').mockResolvedValue();
    vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue(null);
    const wrapper = mountSpace();
    await flushPromises();
    GET.mockClear();
    loadSummaries.mockClear();
    GET.mockResolvedValueOnce({ error: { hint: 'not found' } });

    vi.useFakeTimers();
    capturedLiveHandlers().onEvent({
      type: EVENT_TYPE.DOCUMENT_CREATED,
      data: { document_id: 'new-document-id', slug: 'new-document', title: 'New document' },
      envelope: { project_id: SANDBOX_PROJECT_ID } as never,
    });
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(GET).toHaveBeenCalledWith('/api/v2/acta/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws: 'atlas', slug: 'new-document' } },
    });
    expect(docs.summariesFor('sandbox').map((item) => item.id)).not.toContain('new-document-id');
    expect(loadSummaries).toHaveBeenCalledWith('atlas', 'sandbox');
    wrapper.unmount();
  });

  it('falls back to the catalog reload when the created-document fetch rejects', async () => {
    setupWorkspace();
    const docs = useDocumentsStore();
    const loadSummaries = vi.spyOn(docs, 'loadSummaries').mockResolvedValue();
    vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue(null);
    const wrapper = mountSpace();
    await flushPromises();
    GET.mockClear();
    loadSummaries.mockClear();
    GET.mockRejectedValueOnce(new Error('desktop transport is unavailable'));

    vi.useFakeTimers();
    capturedLiveHandlers().onEvent({
      type: EVENT_TYPE.DOCUMENT_CREATED,
      data: { document_id: 'new-document-id', slug: 'new-document', title: 'New document' },
      envelope: { project_id: SANDBOX_PROJECT_ID } as never,
    });
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(loadSummaries).toHaveBeenCalledWith('atlas', 'sandbox');
    wrapper.unmount();
  });

  it('falls back to the catalog reload when the created-document reconciliation itself throws', async () => {
    setupWorkspace();
    const docs = useDocumentsStore();
    const loadSummaries = vi.spyOn(docs, 'loadSummaries').mockResolvedValue();
    vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue(null);
    const wrapper = mountSpace();
    await flushPromises();
    loadSummaries.mockClear();
    vi.spyOn(docs, 'fetchSummary').mockRejectedValue(new Error('boom'));
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});

    vi.useFakeTimers();
    capturedLiveHandlers().onEvent({
      type: EVENT_TYPE.DOCUMENT_CREATED,
      data: { document_id: 'new-document-id', slug: 'new-document', title: 'New document' },
      envelope: { project_id: SANDBOX_PROJECT_ID } as never,
    });
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(loadSummaries).toHaveBeenCalledWith('atlas', 'sandbox');
    expect(consoleError).toHaveBeenCalledOnce();
    wrapper.unmount();
  });

  it('falls back to the catalog reload when the updated-document reconciliation throws', async () => {
    const { wrapper, docs, loadSummaries } = await mountWithListedDocument();
    vi.spyOn(docs, 'fetchSummary').mockRejectedValue(new Error('boom'));
    vi.spyOn(console, 'error').mockImplementation(() => {});

    vi.useFakeTimers();
    capturedLiveHandlers().onEvent(documentUpdatedFrame(OTHER_USER_ID, 'Old title-document'));
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(loadSummaries).toHaveBeenCalledWith('atlas', 'sandbox');
    wrapper.unmount();
  });

  it('keeps a live-created document when a catalog load that started before the event lands with the old list', async () => {
    const store = new MemoryCacheStore();
    configureCatalogRuntime(store);
    const stale = catalog('Folder', 'Existing document');
    let resolveSummaries: (value: { data: { items: Catalog['summaries']; has_more: boolean } }) => void =
      () => {};
    GET.mockImplementation(async (path: string) => {
      if (path === '/api/v2/acta/workspaces/{ws}/documents/{slug}') {
        return {
          data: {
            id: 'new-document-id',
            slug: 'new-document',
            title: 'New document',
            folder_id: null,
            head_seq: 1,
            updated_at: '2026-01-02T00:00:00Z',
            project_id: SANDBOX_PROJECT_ID,
            workspace_id: WORKSPACE_ID,
            content: '',
            frontmatter: {},
            head_revision_id: 'revision-id',
            created_at: '2026-01-02T00:00:00Z',
          },
          error: undefined,
        };
      }
      if (path.endsWith('/documents')) {
        return new Promise((resolve) => {
          resolveSummaries = resolve;
        });
      }
      return { data: { items: path.endsWith('/folders') ? stale.folders : [], has_more: false } };
    });
    setupWorkspace();

    const wrapper = mountSpace();
    await flushPromises();

    capturedLiveHandlers().onEvent({
      type: EVENT_TYPE.DOCUMENT_CREATED,
      data: { document_id: 'new-document-id', slug: 'new-document', title: 'New document' },
      envelope: { project_id: SANDBOX_PROJECT_ID } as never,
    });
    await flushPromises();
    expect(
      useDocumentsStore()
        .summariesFor('sandbox')
        .map((item) => item.id),
    ).toContain('new-document-id');

    resolveSummaries({ data: { items: stale.summaries, has_more: false } });
    await flushPromises();

    expect(
      useDocumentsStore()
        .summariesFor('sandbox')
        .map((item) => item.id),
    ).toEqual(['Existing document-document', 'new-document-id']);
    expect(wrapper.text()).toContain('New document');
    wrapper.unmount();
  });

  it('stops replaying once the pending live mutations exceed the cap and schedules a reload instead', async () => {
    const store = new MemoryCacheStore();
    configureCatalogRuntime(store);
    const stale = catalog('Folder', 'Existing document');
    let resolveSummaries: (value: { data: { items: Catalog['summaries']; has_more: boolean } }) => void =
      () => {};
    let summariesCalls = 0;
    GET.mockImplementation(async (path: string) => {
      if (path.endsWith('/documents')) {
        summariesCalls += 1;
        if (summariesCalls === 1) {
          return new Promise((resolve) => {
            resolveSummaries = resolve;
          });
        }
        return { data: { items: stale.summaries, has_more: false }, error: undefined };
      }
      return { data: { items: path.endsWith('/folders') ? stale.folders : [], has_more: false } };
    });
    setupWorkspace();
    const docs = useDocumentsStore();
    const removeSummaryById = vi.spyOn(docs, 'removeSummaryById');

    const wrapper = mountSpace();
    await flushPromises();

    vi.useFakeTimers();
    const handlers = capturedLiveHandlers();
    for (let i = 0; i < 65; i += 1) {
      handlers.onEvent({
        type: EVENT_TYPE.DOCUMENT_DELETED,
        data: { document_id: `gone-${i}` },
        envelope: { project_id: SANDBOX_PROJECT_ID } as never,
      });
    }
    expect(removeSummaryById).toHaveBeenCalledTimes(65);

    resolveSummaries({ data: { items: stale.summaries, has_more: false } });
    await vi.advanceTimersByTimeAsync(0);
    expect(removeSummaryById).toHaveBeenCalledTimes(65);
    expect(summariesCalls).toBe(1);

    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();
    await flushPromises();

    expect(summariesCalls).toBe(2);
    expect(removeSummaryById).toHaveBeenCalledTimes(65);
    wrapper.unmount();
  });

  it('ignores a created event for another project', async () => {
    setupWorkspace();
    const loadSummaries = vi.spyOn(useDocumentsStore(), 'loadSummaries').mockResolvedValue();
    vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue(null);
    const wrapper = mountSpace();
    await flushPromises();
    GET.mockClear();
    loadSummaries.mockClear();

    vi.useFakeTimers();
    capturedLiveHandlers().onEvent({
      type: EVENT_TYPE.DOCUMENT_CREATED,
      data: { document_id: 'other-document-id', slug: 'other-document', title: 'Other document' },
      envelope: { project_id: 'a-different-project' } as never,
    });
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(GET).not.toHaveBeenCalled();
    expect(loadSummaries).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('refreshes the catalog atomically for document events and resync', async () => {
    setupWorkspace();
    const docs = useDocumentsStore();
    const folders = useFoldersStore();
    const loadSummaries = vi.spyOn(docs, 'loadSummaries').mockResolvedValue();
    const loadFolders = vi.spyOn(folders, 'load').mockResolvedValue();

    const wrapper = mountSpace();
    await wrapper.vm.$nextTick();

    loadSummaries.mockClear();
    loadFolders.mockClear();

    const handlers = capturedLiveHandlers();
    vi.useFakeTimers();
    handlers.onEvent({ type: 'document.updated', data: {}, envelope: {} as never });
    handlers.onEvent({ type: 'task.updated', data: {}, envelope: {} as never });
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(loadSummaries).toHaveBeenCalledTimes(1);
    expect(loadSummaries).toHaveBeenCalledWith('atlas', 'sandbox');

    handlers.onResync?.('desync');
    await wrapper.vm.$nextTick();

    expect(loadFolders).toHaveBeenCalledWith('atlas', 'sandbox');
    expect(loadSummaries).toHaveBeenLastCalledWith('atlas', 'sandbox');
    wrapper.unmount();
  });

  it('removes a deleted document from the tree without refetching the catalog', async () => {
    setupWorkspace();
    const docs = useDocumentsStore();
    const loadSummaries = vi.spyOn(docs, 'loadSummaries').mockResolvedValue();
    vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue(null);
    const wrapper = mountSpace();
    await flushPromises();

    docs.publishSummariesForProject('sandbox', catalog('Folder', 'Doomed note').summaries);
    GET.mockClear();
    loadSummaries.mockClear();

    vi.useFakeTimers();
    capturedLiveHandlers().onEvent({
      type: EVENT_TYPE.DOCUMENT_DELETED,
      data: { document_id: 'Doomed note-document' },
      envelope: { project_id: SANDBOX_PROJECT_ID } as never,
    });
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(docs.summariesFor('sandbox')).toEqual([]);
    expect(GET).not.toHaveBeenCalled();
    expect(loadSummaries).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('re-parents a moved document in place without refetching the catalog', async () => {
    setupWorkspace();
    const docs = useDocumentsStore();
    const loadSummaries = vi.spyOn(docs, 'loadSummaries').mockResolvedValue();
    vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue(null);
    const wrapper = mountSpace();
    await flushPromises();

    docs.publishSummariesForProject('sandbox', catalog('Folder', 'Travelling note').summaries);
    GET.mockClear();
    loadSummaries.mockClear();

    vi.useFakeTimers();
    capturedLiveHandlers().onEvent({
      type: EVENT_TYPE.DOCUMENT_MOVED,
      data: {
        document_id: 'Travelling note-document',
        project_id: SANDBOX_PROJECT_ID,
        to_folder_id: 'Folder-folder',
      },
      envelope: { project_id: SANDBOX_PROJECT_ID } as never,
    });
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(docs.summariesFor('sandbox').map((item) => item.folder_id)).toEqual(['Folder-folder']);
    expect(GET).not.toHaveBeenCalled();
    expect(loadSummaries).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('refreshes only the touched document row on document.updated, not the whole catalog', async () => {
    setupWorkspace();
    const docs = useDocumentsStore();
    const loadSummaries = vi.spyOn(docs, 'loadSummaries').mockResolvedValue();
    vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue(null);
    const wrapper = mountSpace();
    await flushPromises();

    docs.publishSummariesForProject('sandbox', catalog('Folder', 'Old title').summaries);
    GET.mockClear();
    loadSummaries.mockClear();
    GET.mockResolvedValue({
      error: undefined,
      data: {
        id: 'Old title-document',
        slug: 'old-title',
        title: 'Renamed live',
        folder_id: null,
        head_seq: 3,
        updated_at: '2026-01-03T00:00:00Z',
        project_id: SANDBOX_PROJECT_ID,
        workspace_id: WORKSPACE_ID,
        content: '',
        frontmatter: {},
        head_revision_id: 'revision-id',
        created_at: '2026-01-02T00:00:00Z',
      },
    });

    capturedLiveHandlers().onEvent({
      type: EVENT_TYPE.DOCUMENT_UPDATED,
      data: { document_id: 'Old title-document', revision_id: 'revision-id', seq: 3 },
      envelope: { project_id: SANDBOX_PROJECT_ID } as never,
    });
    await flushPromises();

    expect(GET).toHaveBeenCalledOnce();
    expect(GET).toHaveBeenCalledWith('/api/v2/acta/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws: 'atlas', slug: 'old-title' } },
    });
    expect(docs.summariesFor('sandbox').map((item) => item.title)).toEqual(['Renamed live']);
    expect(loadSummaries).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('ignores a document.updated for a document this space does not list', async () => {
    setupWorkspace();
    const docs = useDocumentsStore();
    const loadSummaries = vi.spyOn(docs, 'loadSummaries').mockResolvedValue();
    vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue(null);
    const wrapper = mountSpace();
    await flushPromises();

    docs.publishSummariesForProject('sandbox', catalog('Folder', 'Mine').summaries);
    GET.mockClear();
    loadSummaries.mockClear();

    vi.useFakeTimers();
    capturedLiveHandlers().onEvent({
      type: EVENT_TYPE.DOCUMENT_UPDATED,
      data: { document_id: 'someone-elses-document', revision_id: 'r', seq: 1 },
      envelope: { project_id: SANDBOX_PROJECT_ID } as never,
    });
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(GET).not.toHaveBeenCalled();
    expect(loadSummaries).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('skips the row refetch for a document.updated frame this client produced', async () => {
    signIn();
    const { wrapper, docs, loadSummaries } = await mountWithListedDocument();

    vi.useFakeTimers();
    capturedLiveHandlers().onEvent(documentUpdatedFrame(SESSION_USER_ID, 'Old title-document'));
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(GET).not.toHaveBeenCalled();
    expect(loadSummaries).not.toHaveBeenCalled();
    expect(docs.summariesFor('sandbox').map((item) => item.title)).toEqual(['Old title']);
    wrapper.unmount();
  });

  it('still refreshes the row for a document.updated frame from another actor', async () => {
    signIn();
    const { wrapper, docs, loadSummaries } = await mountWithListedDocument();

    capturedLiveHandlers().onEvent(documentUpdatedFrame(OTHER_USER_ID, 'Old title-document'));
    await flushPromises();

    expect(GET).toHaveBeenCalledOnce();
    expect(docs.summariesFor('sandbox').map((item) => item.title)).toEqual(['Renamed live']);
    expect(loadSummaries).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('still reloads the catalog for a self document.updated frame carrying no document id', async () => {
    signIn();
    const { wrapper, loadSummaries } = await mountWithListedDocument();

    vi.useFakeTimers();
    capturedLiveHandlers().onEvent(documentUpdatedFrame(SESSION_USER_ID, null));
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(loadSummaries).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it.each([
    EVENT_TYPE.BOARD_CREATED,
    EVENT_TYPE.BOARD_UPDATED,
  ])('refreshes the catalog on a %s live event it cannot place from the payload', async (eventType) => {
    setupWorkspace();
    const docs = useDocumentsStore();
    vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    const loadSummaries = vi.spyOn(docs, 'loadSummaries').mockResolvedValue();

    const wrapper = mountSpace();
    await wrapper.vm.$nextTick();
    loadSummaries.mockClear();

    const handlers = capturedLiveHandlers();
    vi.useFakeTimers();
    handlers.onEvent({ type: eventType, data: {}, envelope: {} as never });
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(loadSummaries).toHaveBeenCalledTimes(1);
    expect(loadSummaries).toHaveBeenCalledWith('atlas', 'sandbox');
    wrapper.unmount();
  });

  it('coalesces a burst of live events into a single catalog reload', async () => {
    setupWorkspace();
    vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    const loadSummaries = vi.spyOn(useDocumentsStore(), 'loadSummaries').mockResolvedValue();
    vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue(null);

    const wrapper = mountSpace();
    await wrapper.vm.$nextTick();
    loadSummaries.mockClear();

    vi.useFakeTimers();
    const handlers = capturedLiveHandlers();
    for (let i = 0; i < 10; i += 1) {
      handlers.onEvent({ type: EVENT_TYPE.DOCUMENT_UPDATED, data: {}, envelope: {} as never });
    }
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(loadSummaries).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it("reloads only for live events targeting this space's project", async () => {
    setupWorkspace();
    vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    const loadSummaries = vi.spyOn(useDocumentsStore(), 'loadSummaries').mockResolvedValue();
    vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue(null);

    const wrapper = mountSpace();
    await wrapper.vm.$nextTick();
    loadSummaries.mockClear();

    const handlers = capturedLiveHandlers();
    vi.useFakeTimers();

    handlers.onEvent({
      type: EVENT_TYPE.DOCUMENT_UPDATED,
      data: {},
      envelope: { project_id: 'a-different-project' } as never,
    });
    await vi.advanceTimersByTimeAsync(2000);
    expect(loadSummaries).not.toHaveBeenCalled();

    handlers.onEvent({
      type: EVENT_TYPE.DOCUMENT_UPDATED,
      data: {},
      envelope: { project_id: SANDBOX_PROJECT_ID } as never,
    });
    await vi.advanceTimersByTimeAsync(2000);
    vi.useRealTimers();

    expect(loadSummaries).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it('surfaces the error state when only the boards load fails in the degraded branch', async () => {
    setResourceCachePrincipal(undefined);
    setupWorkspace();
    vi.spyOn(useFoldersStore(), 'load').mockResolvedValue();
    vi.spyOn(useDocumentsStore(), 'loadSummaries').mockResolvedValue();
    vi.spyOn(useBoardsStore(), 'loadBoardsForProject').mockResolvedValue('Failed to load boards');

    const wrapper = mountSpace();
    await flushPromises();

    expect(wrapper.text()).toContain('Couldn’t load notes');
    expect(wrapper.findComponent(NotesTree).exists()).toBe(false);
    wrapper.unmount();
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
    GET.mockResolvedValue({ data: { items: [], has_more: false }, error: undefined });
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveFolders = resolve;
      }),
    ).mockReturnValueOnce(
      new Promise((resolve) => {
        resolveSummaries = resolve;
      }),
    );

    setupWorkspace();
    const wrapper = mountSpace();
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
    wrapper.unmount();
  });

  it('shows the loader and starts the initial network request while an empty cache lookup is pending', async () => {
    const store = new MemoryCacheStore();
    let resolveCache: (() => void) | undefined;
    vi.spyOn(store, 'get').mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveCache = () => resolve(null);
        }),
    );
    configureCatalogRuntime(store);
    GET.mockReturnValue(new Promise(() => {}));
    setupWorkspace();

    const wrapper = mountSpace();
    await wrapper.vm.$nextTick();

    expect(wrapper.text()).toContain('Loading notes…');
    expect(GET).toHaveBeenCalledWith(
      '/api/v2/acta/workspaces/{ws}/projects/{project_slug}/folders',
      expect.anything(),
    );
    expect(GET).toHaveBeenCalledWith(
      '/api/v2/acta/workspaces/{ws}/projects/{project_slug}/documents',
      expect.anything(),
    );
    expect(GET).toHaveBeenCalledWith(
      '/api/v2/acta/workspaces/{ws}/projects/{project_slug}/boards',
      expect.anything(),
    );

    resolveCache?.();
    wrapper.unmount();
  });

  it('does not let a stale cached composite overwrite the store refresh after a successful rename', async () => {
    const store = new MemoryCacheStore();
    const stale = catalog('Existing folder', 'Old title');
    const renamed = catalog('Existing folder', 'Renamed title');
    seedCatalog(store, 'sandbox', stale);
    configureCatalogRuntime(store);

    let renamedOnServer = false;
    PATCH.mockImplementationOnce(async () => {
      renamedOnServer = true;
      return { data: {}, error: undefined };
    });
    GET.mockImplementation(async (path: string) => ({
      data: {
        items: path.endsWith('/folders')
          ? stale.folders
          : path.endsWith('/boards')
            ? []
            : renamedOnServer
              ? renamed.summaries
              : stale.summaries,
        has_more: false,
      },
      error: undefined,
    }));
    setupWorkspace();
    const wrapper = mountSpace();
    await flushPromises();

    wrapper.findComponent(NotesTree).vm.$emit('rename-doc', 'old-title', 'Renamed title');
    await flushPromises();
    await wrapper.vm.$nextTick();

    expect(
      useDocumentsStore()
        .summariesFor('sandbox')
        .map((summary) => summary.title),
    ).toEqual(['Renamed title']);
    GET.mockReturnValueOnce(new Promise(() => {}))
      .mockReturnValueOnce(new Promise(() => {}))
      .mockReturnValueOnce(new Promise(() => {}));
    capturedLiveHandlers().onResync?.('desync');
    await flushPromises();
    await wrapper.vm.$nextTick();

    expect(
      useDocumentsStore()
        .summariesFor('sandbox')
        .map((summary) => summary.title),
    ).toEqual(['Renamed title']);
    wrapper.unmount();
  });

  it('synchronously clears the prior principal catalog before loading under the next principal', async () => {
    const priorPrincipal = PRINCIPAL;
    const nextPrincipal = 'user:018f4abc-1234-7abc-8def-0123456789ad';
    const store = new MemoryCacheStore();
    seedCatalog(store, 'sandbox', catalog('Prior folder', 'Prior document'));
    configureCatalogRuntime(store);
    GET.mockReturnValue(new Promise(() => {}));
    setupWorkspace();

    const wrapper = mountSpace();
    await flushPromises();
    expect(wrapper.text()).toContain('Prior document');

    setResourceCachePrincipal(nextPrincipal);
    await wrapper.vm.$nextTick();

    expect(wrapper.text()).not.toContain('Prior folder');
    expect(wrapper.text()).not.toContain('Prior document');
    expect(wrapper.text()).toContain('Loading notes…');
    expect(priorPrincipal).not.toBe(nextPrincipal);
    wrapper.unmount();
  });

  it('clears the tree while the principal is purged and reloads the catalog once a principal returns', async () => {
    const store = new MemoryCacheStore();
    configureCatalogRuntime(store);
    const fresh = catalog('Fresh folder', 'Fresh document');
    GET.mockImplementation(async (path: string) => ({
      data: {
        items: path.endsWith('/folders') ? fresh.folders : path.endsWith('/boards') ? [] : fresh.summaries,
        has_more: false,
      },
      error: undefined,
    }));
    setupWorkspace();

    const wrapper = mountSpace();
    await flushPromises();
    expect(wrapper.text()).toContain('Fresh document');
    GET.mockClear();

    const purging = blockAndPurgeResourceCache();
    setResourceCachePrincipal(undefined);
    await purging;
    await flushPromises();

    expect(useDocumentsStore().summariesFor('sandbox')).toEqual([]);
    expect(wrapper.text()).not.toContain('Fresh document');
    expect(GET).not.toHaveBeenCalled();

    setResourceCachePrincipal(PRINCIPAL);
    allowResourceCache();
    await flushPromises();

    expect(GET).toHaveBeenCalledWith(
      '/api/v2/acta/workspaces/{ws}/projects/{project_slug}/documents',
      expect.anything(),
    );
    expect(
      useDocumentsStore()
        .summariesFor('sandbox')
        .map((summary) => summary.title),
    ).toEqual(['Fresh document']);
    expect(wrapper.text()).toContain('Fresh document');
    wrapper.unmount();
  });

  it('uses the empty loader and error lifecycle when catalog caching has no usable key', async () => {
    vi.restoreAllMocks();
    GET.mockReset();
    GET.mockResolvedValue({ data: { items: [] }, error: undefined });
    setResourceCachePrincipal(undefined);
    setupWorkspace();
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

    const wrapper = mountSpace();
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
    wrapper.unmount();
  });

  it('routes sidebar project deletion through the shared lifecycle reconciliation', async () => {
    const store = new MemoryCacheStore();
    configureCatalogRuntime(store);
    const workspace = setupWorkspace();
    vi.spyOn(workspace, 'deleteProject').mockResolvedValue(true);
    const tabs = useNotesTabsStore();
    tabs.open('atlas', { kind: 'doc', id: 'doc-1' }, 'Document', 'sandbox');
    tabs.open('atlas', { kind: 'board', id: 'board-1' }, 'Board', 'sandbox');
    useLastViewedStore().record('atlas', { name: 'notes', params: { slug: 'doc-1' } });

    const wrapper = mountSpace();
    const vm = wrapper.vm as unknown as { deleteProject: () => Promise<void> };
    await vm.deleteProject();

    expect(workspace.deleteProject).toHaveBeenCalledWith('atlas', 'sandbox');
    expect(tabs.tabs('atlas')).toEqual([]);
    expect(useLastViewedStore().forWorkspace('atlas')).toBeNull();
    wrapper.unmount();
  });
});
