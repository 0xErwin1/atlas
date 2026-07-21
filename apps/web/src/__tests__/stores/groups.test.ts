import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { deferred } from '@/__tests__/deferred';

const { GET, POST, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST, DELETE },
}));

import { useGroupsStore } from '@/stores/groups';

const group = (id: string, name: string) => ({
  id,
  name,
  workspace_id: 'ws1',
  created_by: 'u1',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const groupMember = (groupId: string, userId: string) => ({
  group_id: groupId,
  user_id: userId,
  created_at: '2026-01-01T00:00:00Z',
});

describe('useGroupsStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('load populates groups from the workspace groups endpoint', async () => {
    GET.mockResolvedValue({ data: [group('g1', 'Engineering')] });

    const store = useGroupsStore();
    await store.load('acme');

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/groups', {
      params: { path: { ws: 'acme' } },
    });
    expect(store.groups).toHaveLength(1);
    expect(store.groups[0]?.name).toBe('Engineering');
    expect(store.error).toBeNull();
  });

  it('load surfaces the API hint on error', async () => {
    GET.mockResolvedValue({ error: { hint: 'not allowed', detail: 'stack' } });

    const store = useGroupsStore();
    await store.load('acme');

    expect(store.error).toBe('not allowed');
    expect(store.error).not.toContain('stack');
  });

  it('create POSTs the name and re-fetches', async () => {
    POST.mockResolvedValue({ data: group('g2', 'Design') });
    GET.mockResolvedValue({ data: [group('g2', 'Design')] });

    const store = useGroupsStore();
    const ok = await store.create('acme', 'Design');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/groups', {
      params: { path: { ws: 'acme' } },
      body: { name: 'Design' },
    });
    expect(GET).toHaveBeenCalledOnce();
  });

  it('create surfaces the API hint on failure and does not re-fetch', async () => {
    POST.mockResolvedValue({ error: { hint: 'name already in use' } });

    const store = useGroupsStore();
    const ok = await store.create('acme', 'Engineering');

    expect(ok).toBe(false);
    expect(store.error).toBe('name already in use');
    expect(GET).not.toHaveBeenCalled();
  });

  it('remove DELETEs the group and drops it from the local list', async () => {
    DELETE.mockResolvedValue({ data: undefined });

    const store = useGroupsStore();
    store.groups = [group('g1', 'Engineering'), group('g2', 'Design')];

    const ok = await store.remove('acme', 'g1');

    expect(ok).toBe(true);
    expect(DELETE).toHaveBeenCalledWith('/api/workspaces/{ws}/groups/{group_id}', {
      params: { path: { ws: 'acme', group_id: 'g1' } },
    });
    expect(store.groups).toHaveLength(1);
    expect(store.groups[0]?.id).toBe('g2');
  });

  it('loadMembers populates members from the group members endpoint', async () => {
    GET.mockResolvedValue({ data: [groupMember('g1', 'u1'), groupMember('g1', 'u2')] });

    const store = useGroupsStore();
    await store.loadMembers('acme', 'g1');

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/groups/{group_id}/members', {
      params: { path: { ws: 'acme', group_id: 'g1' } },
    });
    expect(store.members).toHaveLength(2);
  });

  it('addMember POSTs the user id and re-fetches members', async () => {
    POST.mockResolvedValue({ data: groupMember('g1', 'u3') });
    GET.mockResolvedValue({ data: [groupMember('g1', 'u3')] });

    const store = useGroupsStore();
    const ok = await store.addMember('acme', 'g1', 'u3');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/groups/{group_id}/members', {
      params: { path: { ws: 'acme', group_id: 'g1' } },
      body: { user_id: 'u3' },
    });
  });

  it('removeMember DELETEs and drops the member locally', async () => {
    DELETE.mockResolvedValue({ data: undefined });

    const store = useGroupsStore();
    store.members = [groupMember('g1', 'u1'), groupMember('g1', 'u2')];

    const ok = await store.removeMember('acme', 'g1', 'u1');

    expect(ok).toBe(true);
    expect(DELETE).toHaveBeenCalledWith('/api/workspaces/{ws}/groups/{group_id}/members/{user_id}', {
      params: { path: { ws: 'acme', group_id: 'g1', user_id: 'u1' } },
    });
    expect(store.members).toHaveLength(1);
    expect(store.members[0]?.user_id).toBe('u2');
  });

  it('does not publish stale groups or expanded members after the workspace resets', async () => {
    const groupsA = deferred<{ data: ReturnType<typeof group>[]; error: undefined }>();
    const membersA = deferred<{ data: ReturnType<typeof groupMember>[]; error: undefined }>();
    GET.mockReturnValueOnce(groupsA.promise).mockReturnValueOnce(membersA.promise);
    GET.mockResolvedValueOnce({
      data: [group('g-b', 'Destination')],
      error: undefined,
    }).mockResolvedValueOnce({ data: [groupMember('g-b', 'u-b')], error: undefined });

    const store = useGroupsStore();
    const loadingGroupsA = store.load('workspace-a');
    const loadingMembersA = store.loadMembers('workspace-a', 'g-a');

    store.resetWorkspace();
    await store.load('workspace-b');
    await store.loadMembers('workspace-b', 'g-b');

    groupsA.resolve({ data: [group('g-a', 'Stale')], error: undefined });
    membersA.resolve({ data: [groupMember('g-a', 'u-a')], error: undefined });
    await Promise.all([loadingGroupsA, loadingMembersA]);

    expect(store.groups.map((item) => item.id)).toEqual(['g-b']);
    expect(store.members.map((item) => item.group_id)).toEqual(['g-b']);
  });
});
