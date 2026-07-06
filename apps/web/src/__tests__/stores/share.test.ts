import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, POST, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST, DELETE },
}));

import { type ShareResource, useShareStore } from '@/stores/share';

const WS_RESOURCE: ShareResource = { kind: 'workspace', ws: 'acme' };
const PROJ_RESOURCE: ShareResource = { kind: 'project', ws: 'acme', projectSlug: 'my-proj' };

const grant = (id: string, type: 'user' | 'api_key', principalId: string, role: string) => ({
  id,
  principal: { type, id: principalId },
  role,
  created_at: '2026-01-01T00:00:00Z',
});

describe('useShareStore — workspace resource (REQ-W26/W27)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('load populates grants from the workspace grants list', async () => {
    GET.mockResolvedValue({
      data: { items: [grant('g1', 'user', 'u1', 'editor')], has_more: false },
    });

    const store = useShareStore();
    await store.load(WS_RESOURCE);

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/grants', {
      params: { path: { ws: 'acme' }, query: { limit: 200 } },
    });
    expect(store.grants).toHaveLength(1);
    expect(store.grants[0]?.id).toBe('g1');
    expect(store.error).toBeNull();
  });

  it('load surfaces the API hint on error, never a raw detail/stack', async () => {
    GET.mockResolvedValue({ error: { hint: 'you cannot manage grants here', detail: 'stack trace' } });

    const store = useShareStore();
    await store.load(WS_RESOURCE);

    expect(store.error).toBe('you cannot manage grants here');
    expect(store.error).not.toContain('stack');
  });

  it('addGrant POSTs a user with admin and re-fetches', async () => {
    POST.mockResolvedValue({ data: grant('g2', 'user', 'u2', 'admin') });
    GET.mockResolvedValue({ data: { items: [grant('g2', 'user', 'u2', 'admin')], has_more: false } });

    const store = useShareStore();
    const ok = await store.addGrant(WS_RESOURCE, { type: 'user', id: 'u2' }, 'admin');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/grants', {
      params: { path: { ws: 'acme' } },
      body: { principal: { type: 'user', id: 'u2' }, role: 'admin' },
    });
    expect(GET).toHaveBeenCalledOnce();
  });

  it('REFUSES to send admin for an api_key agent — guard blocks before the network call (E03)', async () => {
    const store = useShareStore();
    const ok = await store.addGrant(WS_RESOURCE, { type: 'api_key', id: 'k1' }, 'admin');

    expect(ok).toBe(false);
    expect(POST).not.toHaveBeenCalled();
    expect(store.error).toMatch(/admin/i);
  });

  it('allows editor for an api_key agent', async () => {
    POST.mockResolvedValue({ data: grant('g3', 'api_key', 'k1', 'editor') });
    GET.mockResolvedValue({ data: { items: [grant('g3', 'api_key', 'k1', 'editor')], has_more: false } });

    const store = useShareStore();
    const ok = await store.addGrant(WS_RESOURCE, { type: 'api_key', id: 'k1' }, 'editor');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledOnce();
  });

  it('changeRole on an existing agent grant cannot escalate to admin', async () => {
    const store = useShareStore();
    store.grants = [grant('g3', 'api_key', 'k1', 'editor')];

    const ok = await store.changeRole(WS_RESOURCE, 'g3', 'admin');

    expect(ok).toBe(false);
    expect(POST).not.toHaveBeenCalled();
    expect(DELETE).not.toHaveBeenCalled();
    expect(store.error).toMatch(/admin/i);
  });

  it('changeRole re-grants the same principal with the new role (upsert) and re-fetches', async () => {
    POST.mockResolvedValue({ data: grant('g1', 'user', 'u1', 'viewer') });
    GET.mockResolvedValue({ data: { items: [grant('g1', 'user', 'u1', 'viewer')], has_more: false } });

    const store = useShareStore();
    store.grants = [grant('g1', 'user', 'u1', 'editor')];

    const ok = await store.changeRole(WS_RESOURCE, 'g1', 'viewer');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/grants', {
      params: { path: { ws: 'acme' } },
      body: { principal: { type: 'user', id: 'u1' }, role: 'viewer' },
    });
  });

  it('removeGrant DELETEs and re-fetches', async () => {
    DELETE.mockResolvedValue({ data: undefined });
    GET.mockResolvedValue({ data: { items: [], has_more: false } });

    const store = useShareStore();
    const ok = await store.removeGrant(WS_RESOURCE, 'g1');

    expect(ok).toBe(true);
    expect(DELETE).toHaveBeenCalledWith('/api/workspaces/{ws}/grants/{grant_id}', {
      params: { path: { ws: 'acme', grant_id: 'g1' } },
    });
    expect(store.grants).toHaveLength(0);
  });

  it('addGrant surfaces the API hint on failure and does not re-fetch', async () => {
    POST.mockResolvedValue({ error: { hint: 'not a member of this workspace' } });

    const store = useShareStore();
    const ok = await store.addGrant(WS_RESOURCE, { type: 'user', id: 'ghost' }, 'editor');

    expect(ok).toBe(false);
    expect(store.error).toBe('not a member of this workspace');
    expect(GET).not.toHaveBeenCalled();
  });

  it('loadMembers populates members from the workspace members endpoint', async () => {
    GET.mockResolvedValue({
      data: [
        { principal_type: 'user', id: 'u1', display: 'Ada Lovelace' },
        { principal_type: 'api_key', id: 'k1', display: 'ci-bot' },
      ],
    });

    const store = useShareStore();
    await store.loadMembers('acme');

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/members', {
      params: { path: { ws: 'acme' } },
    });
    expect(store.members).toHaveLength(2);
    expect(store.members[0]?.display).toBe('Ada Lovelace');
    expect(store.members[1]?.principal_type).toBe('api_key');
    expect(store.error).toBeNull();
  });

  it('loadMembers surfaces the API hint on error and leaves members untouched', async () => {
    GET.mockResolvedValue({ error: { hint: 'not a member of this workspace', detail: 'stack' } });

    const store = useShareStore();
    await store.loadMembers('acme');

    expect(store.error).toBe('not a member of this workspace');
    expect(store.error).not.toContain('stack');
    expect(store.members).toHaveLength(0);
  });

  it('addGrant still refuses admin for an api_key principal resolved from the member list (E03 cap)', async () => {
    const store = useShareStore();
    store.members = [{ principal_type: 'api_key', id: 'k2', display: 'ci-bot' }];

    const member = store.members[0];
    const ok = await store.addGrant(
      WS_RESOURCE,
      { type: member?.principal_type ?? '', id: member?.id ?? '' },
      'admin',
    );

    expect(ok).toBe(false);
    expect(POST).not.toHaveBeenCalled();
    expect(store.error).toMatch(/admin/i);
  });
});

describe('useShareStore — project resource dispatch', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('load calls the project grants endpoint when resource is a project', async () => {
    GET.mockResolvedValue({
      data: { items: [grant('g1', 'user', 'u1', 'editor')], has_more: false },
    });

    const store = useShareStore();
    await store.load(PROJ_RESOURCE);

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/projects/{project_slug}/grants', {
      params: { path: { ws: 'acme', project_slug: 'my-proj' }, query: { limit: 200 } },
    });
    expect(store.grants).toHaveLength(1);
  });

  it('addGrant POSTs to project grants endpoint for a project resource', async () => {
    POST.mockResolvedValue({ data: grant('g2', 'user', 'u2', 'editor') });
    GET.mockResolvedValue({ data: { items: [grant('g2', 'user', 'u2', 'editor')], has_more: false } });

    const store = useShareStore();
    const ok = await store.addGrant(PROJ_RESOURCE, { type: 'user', id: 'u2' }, 'editor');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/projects/{project_slug}/grants', {
      params: { path: { ws: 'acme', project_slug: 'my-proj' } },
      body: { principal: { type: 'user', id: 'u2' }, role: 'editor' },
    });
  });

  it('addGrant REFUSES admin for api_key even on project resource (E03 cap)', async () => {
    const store = useShareStore();
    const ok = await store.addGrant(PROJ_RESOURCE, { type: 'api_key', id: 'k1' }, 'admin');

    expect(ok).toBe(false);
    expect(POST).not.toHaveBeenCalled();
    expect(store.error).toMatch(/admin/i);
  });

  it('removeGrant DELETEs via project endpoint and re-fetches', async () => {
    DELETE.mockResolvedValue({ data: undefined });
    GET.mockResolvedValue({ data: { items: [], has_more: false } });

    const store = useShareStore();
    const ok = await store.removeGrant(PROJ_RESOURCE, 'g5');

    expect(ok).toBe(true);
    expect(DELETE).toHaveBeenCalledWith('/api/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}', {
      params: { path: { ws: 'acme', project_slug: 'my-proj', grant_id: 'g5' } },
    });
  });

  it('allows editor for api_key on project resource', async () => {
    POST.mockResolvedValue({ data: grant('g6', 'api_key', 'k1', 'editor') });
    GET.mockResolvedValue({ data: { items: [grant('g6', 'api_key', 'k1', 'editor')], has_more: false } });

    const store = useShareStore();
    const ok = await store.addGrant(PROJ_RESOURCE, { type: 'api_key', id: 'k1' }, 'editor');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/projects/{project_slug}/grants', {
      params: { path: { ws: 'acme', project_slug: 'my-proj' } },
      body: { principal: { type: 'api_key', id: 'k1' }, role: 'editor' },
    });
  });
});
