import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET: vi.fn(),
    POST: vi.fn(),
  },
}));

import { wrappedClient } from '@/api/wrapper';
import { useAuthStore } from '@/stores/auth';

const mockGet = wrappedClient.GET as ReturnType<typeof vi.fn>;
const mockPost = wrappedClient.POST as ReturnType<typeof vi.fn>;

const meOk = (username: string, principal_type = 'user') =>
  Promise.resolve({ data: { principal_type, username }, error: undefined });

const meErr = () =>
  Promise.resolve({
    data: undefined,
    error: { type: 'urn:atlas:error:auth-failed', title: 'Unauthorized', status: 401 },
  });

const postOk = () => Promise.resolve({ data: {}, error: undefined });

const postErr = (status = 401) =>
  Promise.resolve({
    data: undefined,
    error: { type: 'urn:atlas:error:auth-failed', title: 'Invalid credentials', status },
  });

describe('useAuthStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('starts unauthenticated with null user', () => {
    const store = useAuthStore();
    expect(store.isAuthenticated).toBe(false);
    expect(store.user).toBeNull();
  });

  it('fetchMe 401 sets isAuthenticated false (REQ-W8)', async () => {
    mockGet.mockReturnValueOnce(meErr());

    const store = useAuthStore();
    await store.fetchMe();

    expect(store.isAuthenticated).toBe(false);
    expect(store.user).toBeNull();
  });

  it('fetchMe 200 hydrates user and sets isAuthenticated true (REQ-W8)', async () => {
    mockGet.mockReturnValueOnce(meOk('alice'));

    const store = useAuthStore();
    await store.fetchMe();

    expect(store.isAuthenticated).toBe(true);
    expect(store.user?.username).toBe('alice');
    expect(store.apiKeyWarning).toBe(false);
  });

  it('api_key principal_type sets apiKeyWarning (REQ-W8)', async () => {
    mockGet.mockReturnValueOnce(meOk('agent', 'api_key'));

    const store = useAuthStore();
    await store.fetchMe();

    expect(store.apiKeyWarning).toBe(true);
  });

  it('logout clears store even when POST fails (REQ-W9)', async () => {
    mockGet.mockReturnValueOnce(meOk('alice'));
    await useAuthStore().fetchMe();

    mockPost.mockRejectedValueOnce(new Error('network error'));

    const store = useAuthStore();
    await store.logout();

    expect(store.isAuthenticated).toBe(false);
    expect(store.user).toBeNull();
  });

  it('login returns ok on 200', async () => {
    mockPost.mockReturnValueOnce(postOk());
    mockGet.mockReturnValueOnce(meOk('bob'));

    const store = useAuthStore();
    const result = await store.login({ username: 'bob', password: 'pass' });

    expect(result.ok).toBe(true);
    expect(store.isAuthenticated).toBe(true);
  });

  it('login returns problem on failure', async () => {
    mockPost.mockReturnValueOnce(postErr(401));

    const store = useAuthStore();
    const result = await store.login({ username: 'bad', password: 'wrong' });

    expect(result.ok).toBe(false);
    expect(store.isAuthenticated).toBe(false);
  });
});
