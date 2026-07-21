import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { deferred } from '@/__tests__/deferred';

const { GET } = vi.hoisted(() => ({
  GET: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import type { AuditEntryDto } from '@/stores/audit';
import { useAuditStore } from '@/stores/audit';

const entry = (id: string, action = 'membership.role_changed'): AuditEntryDto =>
  ({
    id,
    action,
    actor: { id: 'u1', type: 'user', display_name: 'Ada' },
    target_type: 'user',
    target_id: 'u2',
    target_label: 'Bob',
    metadata: { old_role: 'member', new_role: 'admin' },
    created_at: '2026-01-01T00:00:00Z',
  }) as unknown as AuditEntryDto;

const page = (items: ReturnType<typeof entry>[], next: string | null, hasMore: boolean) => ({
  data: { items, next_cursor: next, has_more: hasMore },
  error: undefined,
});

describe('useAuditStore — workspace feed', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('loadWorkspace fetches the first page with no filters and replaces entries', async () => {
    GET.mockResolvedValueOnce(page([entry('a1'), entry('a2')], 'cur1', true));

    const store = useAuditStore();
    await store.loadWorkspace('acme');

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/audit', {
      params: { path: { ws: 'acme' }, query: {} },
    });
    expect(store.entries).toHaveLength(2);
    expect(store.cursor).toBe('cur1');
    expect(store.hasMore).toBe(true);
  });

  it('sends actor and action filters when set', async () => {
    GET.mockResolvedValueOnce(page([], null, false));

    const store = useAuditStore();
    store.setActor('api_key');
    store.setAction('grant.created');
    await store.loadWorkspace('acme');

    const call = GET.mock.calls[0]?.[1] as { params: { query: Record<string, string> } };
    expect(call.params.query.actor).toBe('api_key');
    expect(call.params.query.action).toBe('grant.created');
  });

  it('sends from/to date bounds when a range is set', async () => {
    GET.mockResolvedValueOnce(page([], null, false));

    const store = useAuditStore();
    store.setRange('2026-01-01T00:00:00.000Z', '2026-01-31T23:59:59.999Z');
    await store.loadWorkspace('acme');

    const call = GET.mock.calls[0]?.[1] as { params: { query: Record<string, string> } };
    expect(call.params.query.from).toBe('2026-01-01T00:00:00.000Z');
    expect(call.params.query.to).toBe('2026-01-31T23:59:59.999Z');
  });

  it('loadMoreWorkspace appends the next page using the stored cursor', async () => {
    const store = useAuditStore();
    store._setForTest({ entries: [entry('a1')], cursor: 'cur1', hasMore: true });

    GET.mockResolvedValueOnce(page([entry('a2')], null, false));
    await store.loadMoreWorkspace('acme');

    const call = GET.mock.calls[0]?.[1] as { params: { query: Record<string, string> } };
    expect(call.params.query.cursor).toBe('cur1');
    expect(store.entries.map((e) => e.id)).toEqual(['a1', 'a2']);
    expect(store.hasMore).toBe(false);
  });

  it('loadMoreWorkspace is a no-op when there is no further page', async () => {
    const store = useAuditStore();
    store._setForTest({ entries: [entry('a1')], cursor: null, hasMore: false });

    await store.loadMoreWorkspace('acme');

    expect(GET).not.toHaveBeenCalled();
  });

  it('surfaces the API hint in error on failure', async () => {
    GET.mockResolvedValueOnce({ data: undefined, error: { hint: 'nope' } });

    const store = useAuditStore();
    await store.loadWorkspace('acme');

    expect(store.error).toBe('nope');
    expect(store.entries).toHaveLength(0);
  });

  it('keeps the destination workspace audit page when a stale page settles after reset', async () => {
    const pageA = deferred<ReturnType<typeof page>>();
    GET.mockReturnValueOnce(pageA.promise).mockResolvedValueOnce(page([entry('b1')], null, false));

    const store = useAuditStore();
    const loadingA = store.loadWorkspace('workspace-a');
    store.resetWorkspace();
    await store.loadWorkspace('workspace-b');

    pageA.resolve(page([entry('a1')], null, false));
    await loadingA;

    expect(store.entries.map((item) => item.id)).toEqual(['b1']);
  });

  it('keeps workspace audit entries when an older platform request settles last', async () => {
    const platformPage = deferred<ReturnType<typeof page>>();
    const workspacePage = deferred<ReturnType<typeof page>>();
    GET.mockReturnValueOnce(platformPage.promise).mockReturnValueOnce(workspacePage.promise);

    const store = useAuditStore();
    const loadingPlatform = store.loadPlatform();
    const loadingWorkspace = store.loadWorkspace('workspace-a');

    workspacePage.resolve(page([entry('workspace-1')], null, false));
    await loadingWorkspace;

    platformPage.resolve(page([entry('platform-1', 'user.disabled')], null, false));
    await loadingPlatform;

    expect(store.entries.map((item) => item.id)).toEqual(['workspace-1']);
  });
});

describe('useAuditStore — platform feed', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('loadPlatform fetches the admin endpoint with no path param', async () => {
    GET.mockResolvedValueOnce(page([entry('p1', 'user.disabled')], null, false));

    const store = useAuditStore();
    await store.loadPlatform();

    expect(GET).toHaveBeenCalledWith('/api/admin/audit', {
      params: { query: {} },
    });
    expect(store.entries).toHaveLength(1);
  });

  it('loadMorePlatform appends using the stored cursor', async () => {
    const store = useAuditStore();
    store._setForTest({ entries: [entry('p1')], cursor: 'pcur', hasMore: true });

    GET.mockResolvedValueOnce(page([entry('p2')], null, false));
    await store.loadMorePlatform();

    const call = GET.mock.calls[0]?.[1] as { params: { query: Record<string, string> } };
    expect(call.params.query.cursor).toBe('pcur');
    expect(store.entries.map((e) => e.id)).toEqual(['p1', 'p2']);
  });

  it('surfaces a platform-specific error message on failure', async () => {
    GET.mockResolvedValueOnce({ data: undefined, error: {} });

    const store = useAuditStore();
    await store.loadPlatform();

    expect(store.error).toBe('Failed to load the platform audit log');
  });

  it('keeps platform audit independent from a workspace reset', async () => {
    GET.mockResolvedValueOnce(page([entry('p1', 'user.disabled')], null, false));

    const store = useAuditStore();
    store.resetWorkspace();
    await store.loadPlatform();

    expect(store.entries.map((item) => item.id)).toEqual(['p1']);
  });

  it('keeps platform audit entries when an older workspace request settles last', async () => {
    const workspacePage = deferred<ReturnType<typeof page>>();
    const platformPage = deferred<ReturnType<typeof page>>();
    GET.mockReturnValueOnce(workspacePage.promise).mockReturnValueOnce(platformPage.promise);

    const store = useAuditStore();
    const loadingWorkspace = store.loadWorkspace('workspace-a');
    const loadingPlatform = store.loadPlatform();

    platformPage.resolve(page([entry('platform-1', 'user.disabled')], null, false));
    await loadingPlatform;

    workspacePage.resolve(page([entry('workspace-1')], null, false));
    await loadingWorkspace;

    expect(store.entries.map((item) => item.id)).toEqual(['platform-1']);
  });
});
