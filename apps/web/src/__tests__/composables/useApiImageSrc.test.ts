import { beforeEach, describe, expect, it, vi } from 'vitest';
import { effectScope } from 'vue';
import { useApiImageSrc } from '@/composables/useApiImageSrc';

const { platformFetch } = vi.hoisted(() => ({ platformFetch: vi.fn() }));

vi.mock('@/platform/fetch', () => ({ fetchThroughPlatform: platformFetch }));

// jsdom's Blob cannot be streamed by a real Response, so the transport is faked
// down to the two members the resolver actually reads.
function imageResponse(): Response {
  return { ok: true, blob: () => Promise.resolve({} as Blob) } as Response;
}

function failedResponse(): Response {
  return { ok: false, blob: () => Promise.reject(new Error('no body')) } as Response;
}

function runInScope<T>(body: () => T): { value: T; stop: () => void } {
  const scope = effectScope();
  const value = scope.run(body) as T;
  return { value, stop: () => scope.stop() };
}

describe('useApiImageSrc', () => {
  let created = 0;
  let revoked: string[] = [];

  beforeEach(() => {
    platformFetch.mockReset();
    created = 0;
    revoked = [];
    URL.createObjectURL = vi.fn(() => {
      created += 1;
      return `blob:${created}`;
    });
    URL.revokeObjectURL = vi.fn((url: string) => {
      revoked.push(url);
    });
  });

  it('passes non-API sources through untouched so external images still load directly', async () => {
    const { value: resolve } = runInScope(() => useApiImageSrc());

    await expect(resolve('https://example.com/a.png')).resolves.toBe('https://example.com/a.png');
    expect(platformFetch).not.toHaveBeenCalled();
  });

  it('loads an API source through the platform so the desktop IPC bridge carries it', async () => {
    platformFetch.mockResolvedValue(imageResponse());
    const { value: resolve } = runInScope(() => useApiImageSrc());

    await expect(resolve('/api/workspaces/acme/tasks/ATL-1/attachments/a/content')).resolves.toBe('blob:1');

    const request = platformFetch.mock.calls[0]?.[0] as Request;
    expect(new URL(request.url).pathname).toBe('/api/workspaces/acme/tasks/ATL-1/attachments/a/content');
  });

  it('fetches each source once and shares the object URL across repeated resolutions', async () => {
    platformFetch.mockResolvedValue(imageResponse());
    const { value: resolve } = runInScope(() => useApiImageSrc());

    const [first, second] = await Promise.all([resolve('/api/a'), resolve('/api/a')]);

    expect(first).toBe('blob:1');
    expect(second).toBe('blob:1');
    expect(platformFetch).toHaveBeenCalledTimes(1);
  });

  it('reports an unresolvable source as null instead of yielding a broken src', async () => {
    platformFetch.mockResolvedValue(failedResponse());
    const { value: resolve } = runInScope(() => useApiImageSrc());

    await expect(resolve('/api/missing')).resolves.toBeNull();
  });

  it('retries a source whose previous load failed', async () => {
    platformFetch.mockResolvedValueOnce(failedResponse()).mockResolvedValueOnce(imageResponse());
    const { value: resolve } = runInScope(() => useApiImageSrc());

    await expect(resolve('/api/a')).resolves.toBeNull();
    await expect(resolve('/api/a')).resolves.toBe('blob:1');
  });

  it('revokes every object URL it created when its owner unmounts', async () => {
    platformFetch.mockResolvedValue(imageResponse());
    const { value: resolve, stop } = runInScope(() => useApiImageSrc());

    await resolve('/api/a');
    await resolve('/api/b');
    stop();

    expect(revoked).toEqual(['blob:1', 'blob:2']);
  });
});
