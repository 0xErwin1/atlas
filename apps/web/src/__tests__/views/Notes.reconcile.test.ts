import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import {
  blockAndPurgeResourceCache,
  configureResourceCacheForTest,
  resourceCacheEpoch,
  setResourceCachePrincipal,
} from '@/cache/cacheRuntime';
import { buildCacheKey, type CacheEnvelope, ResourceCache } from '@/cache/resourceCache';
import { EVENT_TYPE } from '@/lib/eventTypes';
import { useDocumentsStore } from '@/stores/documents';
import { useWorkspaceStore } from '@/stores/workspace';
import Notes, { isDocumentMissingError, planDocumentReconcile } from '@/views/Notes.vue';

describe('planDocumentReconcile', () => {
  it('applies the remote revision when the open note is clean', () => {
    expect(planDocumentReconcile('doc-1', 'doc-1', false)).toBe('apply');
  });

  it('keeps local edits and flags a pending remote change when the open note is dirty', () => {
    expect(planDocumentReconcile('doc-1', 'doc-1', true)).toBe('keep-and-flag');
  });

  it('ignores an event for a document other than the open note', () => {
    expect(planDocumentReconcile('doc-1', 'doc-2', false)).toBe('ignore');
    expect(planDocumentReconcile('doc-1', 'doc-2', true)).toBe('ignore');
  });

  it('ignores an event when no note is open', () => {
    expect(planDocumentReconcile(null, 'doc-2', false)).toBe('ignore');
  });

  it('ignores an event carrying no document id', () => {
    expect(planDocumentReconcile('doc-1', null, false)).toBe('ignore');
  });
});

describe('isDocumentMissingError', () => {
  it('recognizes a 404 status as a missing document', () => {
    expect(isDocumentMissingError({ status: 404 })).toBe(true);
  });

  it('does not treat other statuses as a missing document', () => {
    expect(isDocumentMissingError({ status: 500 })).toBe(false);
    expect(isDocumentMissingError({ status: 0 })).toBe(false);
  });

  it('does not treat an unshaped error as a missing document', () => {
    expect(isDocumentMissingError(new Error('boom'))).toBe(false);
    expect(isDocumentMissingError(undefined)).toBe(false);
  });
});

// --- Stateful reconcile wiring (mounted Notes.vue) ---------------------------
//
// The tests above only exercise the pure decision helpers. The following mount
// the real component to prove the *wiring* around those helpers: that a clean
// resync/`document.updated` actually reaches `applyLoadedDocument`, that a dirty
// note's guard really blocks it, that scoping by `envelope.document_id` really
// ignores foreign notes, and that a 404 mid-reconcile really reaches the shared
// missing-document recovery.

type CapturedLiveHandlers = { onEvent: (event: unknown) => void; onResync: () => void };

const route = vi.hoisted(() => ({ params: { slug: 'note-a' } }));
const router = vi.hoisted(() => ({ push: vi.fn(), replace: vi.fn() }));
const liveHandlers = vi.hoisted<{ current: CapturedLiveHandlers | null }>(() => ({ current: null }));
const { mockGet, mockPut } = vi.hoisted(() => ({ mockGet: vi.fn(), mockPut: vi.fn() }));

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

vi.mock('vue-router', () => ({
  useRoute: () => route,
  useRouter: () => router,
  onBeforeRouteLeave: vi.fn(),
  onBeforeRouteUpdate: vi.fn(),
}));

vi.mock('@/composables/useBreakpoint', () => ({ useBreakpoint: () => ({ isMobile: false }) }));

vi.mock('@/composables/useDocumentPresence', () => ({
  useDocumentPresence: () => ({ actors: [], apply: vi.fn() }),
}));

vi.mock('@/composables/useLiveUpdates', () => ({
  useLiveUpdates: vi.fn((_ws: unknown, handlers: CapturedLiveHandlers) => {
    liveHandlers.current = handlers;
  }),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET: mockGet, PUT: mockPut },
}));

type DocFixture =
  | { kind: 'ok'; id: string; slug: string; content: string; headRevisionId: string }
  | { kind: 'error'; status: number };

let docFixture: DocFixture = {
  kind: 'ok',
  id: 'doc-1',
  slug: 'note-a',
  content: 'Hello',
  headRevisionId: 'rev-1',
};

const NoteEditorStub = {
  name: 'NoteEditor',
  props: ['body', 'wikilinkTitles', 'uploadImage', 'mode', 'reading'],
  emits: ['change', 'navigate-wikilink', 'wikilink-query'],
  methods: {
    currentMarkdown(this: { body: string }) {
      return this.body;
    },
  },
  template: '<div data-test="note-editor">{{ body }}</div>',
};

const EditorToolbarStub = {
  name: 'EditorToolbar',
  props: ['breadcrumbs', 'dirty'],
  template: '<div><slot /></div>',
};

function mountNotes() {
  return mount(Notes, {
    global: {
      stubs: {
        AppShell: {
          template: '<div><slot name="sidebar" /><slot /><slot name="inspector-backlinks" /></div>',
        },
        NotesSidebar: true,
        BacklinksPanel: true,
        CasConflictView: true,
        DocumentComments: true,
        HistoryPanel: true,
        PropertiesEditor: true,
        PropertiesPanel: true,
        SharePanel: true,
        WikiLinkSuggest: true,
        EmptyState: true,
        ErrorState: true,
        LoadingState: true,
        Icon: true,
        PresenceAvatars: true,
        TabStrip: true,
        EditorToolbar: EditorToolbarStub,
        NoteEditor: NoteEditorStub,
      },
    },
  });
}

function documentUpdatedEvent(documentId: string | null) {
  return {
    type: EVENT_TYPE.DOCUMENT_UPDATED,
    data: {},
    envelope: {
      id: 'evt-1',
      event_type: EVENT_TYPE.DOCUMENT_UPDATED,
      version: 1,
      source: 'test',
      workspace_id: 'ws-1',
      document_id: documentId,
      occurred_at: '2026-01-01T00:00:00Z',
      actor: { type: 'user', id: 'u1' },
      data: {},
    },
  };
}

async function settle(): Promise<void> {
  await vi.advanceTimersByTimeAsync(0);
  await nextTick();
}

describe('Notes.vue open-note reconcile wiring', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    localStorage.clear();
    vi.useFakeTimers();
    router.push.mockReset();
    router.replace.mockReset();
    liveHandlers.current = null;
    route.params.slug = 'note-a';

    docFixture = { kind: 'ok', id: 'doc-1', slug: 'note-a', content: 'Hello', headRevisionId: 'rev-1' };

    mockGet.mockReset();
    mockGet.mockImplementation((url: string) => {
      if (url === '/api/workspaces/{ws}/documents/{slug}/backlinks') {
        return Promise.resolve({ data: { items: [], has_more: false }, error: undefined });
      }
      if (url === '/api/workspaces/{ws}/documents/{slug}') {
        if (docFixture.kind === 'error') {
          return Promise.resolve({
            data: undefined,
            error: { title: 'Problem' },
            response: { status: docFixture.status },
          });
        }
        return Promise.resolve({
          data: {
            id: docFixture.id,
            slug: docFixture.slug,
            content: docFixture.content,
            head_revision_id: docFixture.headRevisionId,
          },
          error: undefined,
        });
      }
      return Promise.resolve({
        data: undefined,
        error: { title: 'Unhandled URL in test' },
        response: { status: 500 },
      });
    });

    mockPut.mockReset();
    mockPut.mockResolvedValue({ data: { head_revision_id: 'rev-put' }, error: undefined });

    useWorkspaceStore().activeWorkspaceSlug = 'acme';
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('applies a clean remote update via applyLoadedDocument and leaves remoteChangesPending false', async () => {
    const wrapper = mountNotes();
    await settle();

    expect(wrapper.get('[data-test="note-editor"]').text()).toBe('Hello');
    expect(wrapper.find('[aria-label="Remote changes pending"]').exists()).toBe(false);

    docFixture = {
      kind: 'ok',
      id: 'doc-1',
      slug: 'note-a',
      content: 'Updated remotely',
      headRevisionId: 'rev-2',
    };
    liveHandlers.current?.onResync();
    await settle();

    expect(wrapper.get('[data-test="note-editor"]').text()).toBe('Updated remotely');
    expect(wrapper.find('[aria-label="Remote changes pending"]').exists()).toBe(false);

    // Prove headRevisionId/baseContent (the CAS base) really advanced to rev-2 —
    // not just body — by letting the next save fire and inspecting what it CASes
    // against. If applyLoadedDocument stopped short of updating headRevisionId,
    // this would still show the stale 'rev-1'.
    const editor = wrapper.findComponent<typeof NoteEditorStub>('[data-test="note-editor"]');
    await editor.vm.$emit('change', 'Edited after reconcile');
    await vi.advanceTimersByTimeAsync(800);
    await settle();

    expect(mockPut).toHaveBeenCalledWith(
      expect.stringContaining('/documents/{slug}/content'),
      expect.objectContaining({
        body: expect.objectContaining({ base_revision_id: 'rev-2' }),
      }),
    );
  });

  it('keeps local edits and flags remoteChangesPending when the open note is dirty, never touching body/baseContent/headRevisionId', async () => {
    const wrapper = mountNotes();
    await settle();

    const editor = wrapper.findComponent<typeof NoteEditorStub>('[data-test="note-editor"]');
    await editor.vm.$emit('change', 'My unsaved edit');
    await settle();

    expect(wrapper.get('[data-test="note-editor"]').text()).toBe('My unsaved edit');
    const getCallsBeforeReconcile = mockGet.mock.calls.length;

    docFixture = {
      kind: 'ok',
      id: 'doc-1',
      slug: 'note-a',
      content: 'Remote edit while dirty',
      headRevisionId: 'rev-9',
    };
    liveHandlers.current?.onEvent(documentUpdatedEvent('doc-1'));
    await settle();

    // Local edits are preserved verbatim: the remote content never overwrote them.
    expect(wrapper.get('[data-test="note-editor"]').text()).toBe('My unsaved edit');
    // keep-and-flag must never re-fetch — that would race the in-flight local edit.
    expect(mockGet.mock.calls.length).toBe(getCallsBeforeReconcile);
    expect(wrapper.find('[aria-label="Remote changes pending"]').exists()).toBe(true);
  });

  it('does not let a same-target cached refresh overwrite dirty editor content or its CAS base', async () => {
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const key = `v1|p=${principal}|w=${workspaceId}|k=note-body|r=note-a|q={}`;
    const now = Date.now();
    const entries = new Map<string, CacheEnvelope<unknown>>([
      [
        key,
        {
          schema: 1,
          key,
          payloadVersion: 1,
          storedAt: now,
          validatedAt: now,
          lastAccessedAt: now,
          retentionExpiresAt: now + 60_000,
          bytes: 128,
          stale: false,
          tags: ['document:note-a'],
          payload: {
            id: 'doc-1',
            body: 'Cached body',
            meta: {},
            headRevisionId: 'cached-revision',
            slug: 'note-a',
          },
        },
      ],
    ]);
    const cache = new ResourceCache({
      store: {
        get: async (entryKey) => (entries.get(entryKey) as never) ?? null,
        putMany: async () => true,
        deleteMany: async () => true,
        clear: async () => true,
      },
    });
    cache.allow();
    configureResourceCacheForTest(cache);
    setResourceCachePrincipal(principal);
    vi.spyOn(useWorkspaceStore(), 'workspaceIdForSlug').mockReturnValue(workspaceId);

    const networkResolvers: Array<(value: unknown) => void> = [];
    mockGet.mockImplementation((url: string) => {
      if (url === '/api/workspaces/{ws}/documents/{slug}/backlinks') {
        return Promise.resolve({ data: { items: [], has_more: false }, error: undefined });
      }
      if (url === '/api/workspaces/{ws}/documents/{slug}') {
        return new Promise((resolve) => {
          networkResolvers.push(resolve);
        });
      }
      return Promise.resolve({ error: { title: 'Unhandled URL in test' }, response: { status: 500 } });
    });

    const wrapper = mountNotes();
    await settle();
    const editor = wrapper.findComponent<typeof NoteEditorStub>('[data-test="note-editor"]');
    expect(editor.text()).toBe('Cached body');
    await editor.vm.$emit('change', 'My unsaved edit');
    await settle();

    networkResolvers[0]?.({
      data: { id: 'doc-1', slug: 'note-a', content: 'Remote body', head_revision_id: 'remote-revision' },
      error: undefined,
    });
    await settle();

    expect(editor.text()).toBe('My unsaved edit');
    await vi.advanceTimersByTimeAsync(800);
    expect(mockPut).toHaveBeenLastCalledWith(
      expect.stringContaining('/documents/{slug}/content'),
      expect.objectContaining({ body: expect.objectContaining({ base_revision_id: 'cached-revision' }) }),
    );
  });

  it('synchronously clears a cached note on a principal change and rejects the prior principal network result', async () => {
    const priorPrincipal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const nextPrincipal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd09';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const key = `v1|p=${priorPrincipal}|w=${workspaceId}|k=note-body|r=note-a|q={}`;
    const now = Date.now();
    const entries = new Map<string, CacheEnvelope<unknown>>([
      [
        key,
        {
          schema: 1,
          key,
          payloadVersion: 1,
          storedAt: now,
          validatedAt: now,
          lastAccessedAt: now,
          retentionExpiresAt: now + 60_000,
          bytes: 128,
          stale: false,
          tags: ['document:note-a'],
          payload: {
            id: 'doc-1',
            body: 'Prior principal cache',
            meta: {},
            headRevisionId: 'prior-revision',
            slug: 'note-a',
          },
        },
      ],
    ]);
    const cache = new ResourceCache({
      store: {
        get: async (entryKey) => (entries.get(entryKey) as never) ?? null,
        putMany: async () => true,
        deleteMany: async () => true,
        clear: async () => true,
      },
    });
    cache.allow();
    configureResourceCacheForTest(cache);
    setResourceCachePrincipal(priorPrincipal);
    vi.spyOn(useWorkspaceStore(), 'workspaceIdForSlug').mockReturnValue(workspaceId);

    const networkResolvers: Array<(value: unknown) => void> = [];
    mockGet.mockImplementation((url: string) => {
      if (url === '/api/workspaces/{ws}/documents/{slug}/backlinks') {
        return Promise.resolve({ data: { items: [], has_more: false }, error: undefined });
      }
      if (url === '/api/workspaces/{ws}/documents/{slug}') {
        return new Promise((resolve) => {
          networkResolvers.push(resolve);
        });
      }
      return Promise.resolve({ error: { title: 'Unhandled URL in test' }, response: { status: 500 } });
    });

    const wrapper = mountNotes();
    await settle();
    expect(wrapper.get('[data-test="note-editor"]').text()).toBe('Prior principal cache');

    setResourceCachePrincipal(nextPrincipal);
    await nextTick();

    expect(wrapper.find('[data-test="note-editor"]').exists()).toBe(false);

    networkResolvers[0]?.({
      data: {
        id: 'doc-1',
        slug: 'note-a',
        content: 'Prior principal network',
        head_revision_id: 'prior-network',
      },
      error: undefined,
    });
    await settle();

    expect(wrapper.find('[data-test="note-editor"]').exists()).toBe(false);
  });

  it('keeps dirty local edits visible across a same-principal cache epoch', async () => {
    const wrapper = mountNotes();
    await settle();
    const editor = wrapper.findComponent<typeof NoteEditorStub>('[data-test="note-editor"]');
    await editor.vm.$emit('change', 'My same-principal edit');
    await settle();

    resourceCacheEpoch.value += 1;
    await nextTick();

    expect(wrapper.get('[data-test="note-editor"]').text()).toBe('My same-principal edit');
  });

  it('retries backlinks through the exact cache-aware request and persists the recovered result', async () => {
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const key = buildCacheKey({
      principal,
      workspaceId,
      resourceKind: 'note-secondary',
      resourceId: 'note-a',
      query: { type: 'backlinks' },
    });
    if (key === null) throw new Error('Expected a canonical backlinks cache key');

    const entries = new Map<string, CacheEnvelope<unknown>>();
    const cache = new ResourceCache({
      store: {
        get: async (entryKey) => (entries.get(entryKey) as never) ?? null,
        putMany: async (newEntries) => {
          for (const entry of newEntries) entries.set(entry.key, entry);
          return true;
        },
        deleteMany: async () => true,
        clear: async () => true,
      },
    });
    cache.allow();
    configureResourceCacheForTest(cache);
    setResourceCachePrincipal(principal);
    vi.spyOn(useWorkspaceStore(), 'workspaceIdForSlug').mockReturnValue(workspaceId);

    let backlinksAttempt = 0;
    mockGet.mockImplementation((url: string) => {
      if (url === '/api/workspaces/{ws}/documents/{slug}/backlinks') {
        backlinksAttempt += 1;
        return Promise.resolve(
          backlinksAttempt === 1
            ? { error: { hint: 'offline' } }
            : {
                data: {
                  items: [
                    {
                      display_title: 'Recovered source',
                      source_document_id: 'source-1',
                      source_slug: 'source-1',
                      source_title: 'Recovered source',
                    },
                  ],
                  has_more: false,
                },
                error: undefined,
              },
        );
      }
      if (url === '/api/workspaces/{ws}/documents/{slug}') {
        return Promise.resolve({
          data: { id: 'doc-1', slug: 'note-a', content: 'Hello', head_revision_id: 'rev-1' },
          error: undefined,
        });
      }
      return Promise.resolve({ error: { title: 'Unhandled URL in test' }, response: { status: 500 } });
    });

    const wrapper = mountNotes();
    await settle();

    await wrapper.findComponent({ name: 'BacklinksPanel' }).vm.$emit('retry');
    await settle();

    expect(useDocumentsStore().backlinks.map((link) => link.source_slug)).toEqual(['source-1']);
    expect((entries.get(key)?.payload as Array<{ source_slug: string }>)[0]?.source_slug).toBe('source-1');
  });

  it('ignores a document.updated event scoped to a different document than the open note', async () => {
    const wrapper = mountNotes();
    await settle();
    const getCallsBefore = mockGet.mock.calls.length;

    liveHandlers.current?.onEvent(documentUpdatedEvent('doc-2'));
    await settle();

    expect(wrapper.get('[data-test="note-editor"]').text()).toBe('Hello');
    expect(mockGet.mock.calls.length).toBe(getCallsBefore);
    expect(wrapper.find('[aria-label="Remote changes pending"]').exists()).toBe(false);
  });

  it('ignores a resync/document.updated signal when no note is open', async () => {
    route.params.slug = '';
    mountNotes();
    await settle();

    liveHandlers.current?.onResync();
    liveHandlers.current?.onEvent(documentUpdatedEvent('doc-1'));
    await settle();

    expect(mockGet).not.toHaveBeenCalled();
  });

  it('recovers via the shared missing-document handler when a reconcile load 404s', async () => {
    mountNotes();
    await settle();

    docFixture = { kind: 'error', status: 404 };
    liveHandlers.current?.onResync();
    await settle();

    expect(router.replace).toHaveBeenCalledTimes(1);
    expect(router.replace).toHaveBeenCalledWith({ name: 'notes' });
  });

  it('does not treat a 401 from the resync load as a missing document', async () => {
    const wrapper = mountNotes();
    await settle();
    const getCallsBefore = mockGet.mock.calls.length;

    docFixture = { kind: 'error', status: 401 };
    liveHandlers.current?.onResync();
    await settle();

    expect(mockGet).toHaveBeenCalledTimes(getCallsBefore + 1);
    expect(mockGet).toHaveBeenLastCalledWith(
      '/api/workspaces/{ws}/documents/{slug}',
      expect.objectContaining({ params: { path: { ws: 'acme', slug: 'note-a' } } }),
    );
    expect(router.replace).not.toHaveBeenCalled();
    expect(wrapper.get('[data-test="note-editor"]').text()).toBe('Hello');
  });

  it.each([403, 404])('retracts the editor before presenting a known denial (%i)', async (status) => {
    const wrapper = mountNotes();
    await settle();

    docFixture = { kind: 'error', status };
    liveHandlers.current?.onResync();
    await settle();

    expect(wrapper.find('[data-test="note-editor"]').exists()).toBe(false);
    if (status === 404) expect(router.replace).toHaveBeenCalledWith({ name: 'notes' });
  });

  it('does not start a replacement GET while logout cache purge is underway', async () => {
    const purge = deferred<boolean>();
    const cache = new ResourceCache({
      store: {
        get: async () => null,
        putMany: async () => true,
        deleteMany: async () => true,
        clear: async () => true,
      },
    });
    cache.allow();
    vi.spyOn(cache, 'purge').mockReturnValue(purge.promise);
    configureResourceCacheForTest(cache);
    setResourceCachePrincipal('user:019ef171-bbcf-7b90-9be6-5dbb382afd08');
    const wrapper = mountNotes();
    await settle();
    const getCallsBeforeLogout = mockGet.mock.calls.length;

    const purging = blockAndPurgeResourceCache();
    setResourceCachePrincipal(undefined);
    await nextTick();

    expect(wrapper.find('[data-test="note-editor"]').exists()).toBe(false);
    expect(mockGet.mock.calls.length).toBe(getCallsBeforeLogout);

    purge.resolve(true);
    await purging;
  });
});
