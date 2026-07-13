import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET: vi.fn(),
    POST: vi.fn(),
  },
}));

vi.mock('@/lib/workspaceLiveUpdates', () => ({
  disposeWorkspaceLiveUpdates: vi.fn(),
  setWorkspaceLiveUpdatesAuthorizationInvalidator: vi.fn(),
}));

import { wrappedClient } from '@/api/wrapper';
import {
  disposeWorkspaceLiveUpdates,
  setWorkspaceLiveUpdatesAuthorizationInvalidator,
} from '@/lib/workspaceLiveUpdates';
import { type MeResponse, useAuthStore } from '@/stores/auth';

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

  it('disposes live updates before clearing authentication state', () => {
    const store = useAuthStore();
    store.user = { username: 'alice' } as MeResponse;
    store.isAuthenticated = true;

    store.clearUser();

    expect(disposeWorkspaceLiveUpdates).toHaveBeenCalledOnce();
    expect(store.user).toBeNull();
    expect(store.isAuthenticated).toBe(false);
  });

  it('registers the default broker invalidator through clearUser', () => {
    const store = useAuthStore();
    store.user = { username: 'alice' } as MeResponse;
    store.isAuthenticated = true;

    const invalidate = vi.mocked(setWorkspaceLiveUpdatesAuthorizationInvalidator).mock.calls[0]?.[0];
    invalidate?.();

    expect(disposeWorkspaceLiveUpdates).toHaveBeenCalledOnce();
    expect(store.isAuthenticated).toBe(false);
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

  it('login surfaces an unreachable problem when the request throws (server down)', async () => {
    mockPost.mockRejectedValueOnce(new Error('Failed to fetch'));

    const store = useAuthStore();
    const result = await store.login({ username: 'bob', password: 'pass' });

    expect(result.ok).toBe(false);
    expect(result.problem?.type).toBe('urn:atlas:error:unreachable');
    expect(result.problem?.hint).toBeTruthy();
  });

  it('login still returns a problem when the error body is empty', async () => {
    mockPost.mockReturnValueOnce(Promise.resolve({ data: undefined, error: undefined }));

    const store = useAuthStore();
    const result = await store.login({ username: 'bob', password: 'pass' });

    expect(result.ok).toBe(false);
    expect(result.problem).toBeTruthy();
  });
});
