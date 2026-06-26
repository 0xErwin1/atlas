import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import { useUsersStore } from '@/stores/users';

describe('useUsersStore — loadMemberships', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('GETs the memberships endpoint and stores a slug -> role map', async () => {
    GET.mockResolvedValue({
      data: [
        { role: 'admin', workspace_name: 'Acme', workspace_slug: 'acme' },
        { role: 'member', workspace_name: 'Beta', workspace_slug: 'beta' },
      ],
      error: undefined,
    });

    const store = useUsersStore();
    const map = await store.loadMemberships('u1');

    expect(GET).toHaveBeenCalledWith('/v1/users/{user_id}/memberships', {
      params: { path: { user_id: 'u1' } },
    });
    expect(map).toEqual({ acme: 'admin', beta: 'member' });
    expect(store.memberships.u1).toEqual({ acme: 'admin', beta: 'member' });
    expect(store.error).toBeNull();
  });

  it('returns an empty map when the user has no memberships', async () => {
    GET.mockResolvedValue({ data: [], error: undefined });

    const store = useUsersStore();
    const map = await store.loadMemberships('u2');

    expect(map).toEqual({});
    expect(store.memberships.u2).toEqual({});
  });

  it('surfaces the API hint and returns null on error', async () => {
    GET.mockResolvedValue({ data: undefined, error: { hint: 'forbidden' } });

    const store = useUsersStore();
    const map = await store.loadMemberships('u3');

    expect(map).toBeNull();
    expect(store.error).toBe('forbidden');
    expect(store.memberships.u3).toBeUndefined();
  });
});
