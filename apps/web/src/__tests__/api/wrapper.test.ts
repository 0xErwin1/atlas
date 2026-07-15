import { describe, expect, it, vi } from 'vitest';
import {
  cacheInvalidationMiddlewareForTest,
  setCacheInvalidationHandler,
  setRequestOutcomeHandler,
  wrappedClient,
} from '@/api/wrapper';

function responseContext(status: number) {
  return {
    request: new Request('https://atlas.test/api/workspaces/workspace-a/documents/doc-a'),
    response: new Response(null, { status }),
  } as Parameters<NonNullable<typeof cacheInvalidationMiddlewareForTest.onResponse>>[0];
}

describe('cache invalidation middleware', () => {
  it('keeps resource failures resource-scoped and preserves the workspace slug for UUID resolution', () => {
    const invalidate = vi.fn();
    setCacheInvalidationHandler(invalidate);

    cacheInvalidationMiddlewareForTest.onResponse?.(responseContext(403));

    expect(invalidate).toHaveBeenCalledWith({
      status: 403,
      scope: 'resource',
      workspaceSlug: 'workspace-a',
      tags: ['document:doc-a'],
    });
  });

  it('scopes a resource 404 to its matching tag instead of clearing all caches', () => {
    const invalidate = vi.fn();
    setCacheInvalidationHandler(invalidate);

    cacheInvalidationMiddlewareForTest.onResponse?.(responseContext(404));

    expect(invalidate).toHaveBeenCalledWith({
      status: 404,
      scope: 'resource',
      workspaceSlug: 'workspace-a',
      tags: ['document:doc-a'],
    });
  });

  it.each([
    'tasks',
    'projects',
    'documents',
    'folders',
    'boards',
  ])('conservatively invalidates the workspace for a cacheable %s collection failure', (collection) => {
    const invalidate = vi.fn();
    setCacheInvalidationHandler(invalidate);
    const context = {
      request: new Request(`https://atlas.test/api/workspaces/workspace-a/${collection}`),
      response: new Response(null, { status: 403 }),
    } as Parameters<NonNullable<typeof cacheInvalidationMiddlewareForTest.onResponse>>[0];

    cacheInvalidationMiddlewareForTest.onResponse?.(context);

    expect(invalidate).toHaveBeenCalledWith({
      status: 403,
      scope: 'workspace',
      workspaceSlug: 'workspace-a',
      tags: [],
    });
  });

  it('does not purge for a successful response', () => {
    const invalidate = vi.fn();
    setCacheInvalidationHandler(invalidate);

    cacheInvalidationMiddlewareForTest.onResponse?.(responseContext(200));

    expect(invalidate).not.toHaveBeenCalled();
  });

  it('fails closed for an unknown resource route instead of inventing a cache tag', () => {
    const invalidate = vi.fn();
    setCacheInvalidationHandler(invalidate);
    const context = {
      request: new Request('https://atlas.test/api/workspaces/workspace-a/unknown-resource/item-a'),
      response: new Response(null, { status: 404 }),
    } as Parameters<NonNullable<typeof cacheInvalidationMiddlewareForTest.onResponse>>[0];

    cacheInvalidationMiddlewareForTest.onResponse?.(context);

    expect(invalidate).toHaveBeenCalledWith({
      status: 404,
      scope: 'none',
      workspaceSlug: 'workspace-a',
      tags: [],
    });
  });

  it('awaits scoped invalidation before completing a protected failure response', async () => {
    let finishInvalidation: (() => void) | undefined;
    const invalidate = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finishInvalidation = resolve;
        }),
    );
    setCacheInvalidationHandler(invalidate);

    const completion = cacheInvalidationMiddlewareForTest.onResponse?.(responseContext(403));
    expect(invalidate).toHaveBeenCalledOnce();
    let completed = false;
    void Promise.resolve(completion).then(() => {
      completed = true;
    });
    await Promise.resolve();
    expect(completed).toBe(false);

    finishInvalidation?.();
    await completion;
  });

  it('drives transport outcomes through the production wrapped client', async () => {
    const outcomes: string[] = [];
    setRequestOutcomeHandler((outcome) => outcomes.push(outcome));
    const fetch = vi.fn().mockResolvedValue(new Response(null, { status: 503 }));
    vi.stubGlobal('fetch', fetch);

    await wrappedClient.GET('/api/auth/me');

    expect(fetch).toHaveBeenCalledOnce();
    expect(outcomes).toEqual(['start', 'failure']);
    vi.unstubAllGlobals();
  });
});
