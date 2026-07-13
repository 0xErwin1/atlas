import { describe, expect, it, vi } from 'vitest';

describe('CSRF wrapper', () => {
  async function invokeMiddleware(method: string): Promise<Request> {
    const { csrfMiddlewareForTest } = await import('../api/wrapper');

    const original = new Request(`http://localhost/api/test`, { method });

    const result = await csrfMiddlewareForTest.onRequest?.({
      request: original,
      schemaPath: '/api/test',
      params: {},
      id: 'test-id',
      options: {
        baseUrl: 'http://localhost',
        parseAs: 'json',
        querySerializer: () => '',
        bodySerializer: (b: unknown) => JSON.stringify(b),
        pathSerializer: (p: string) => p,
        fetch: globalThis.fetch,
      },
    });

    return result instanceof Request ? result : original;
  }

  it('adds X-Atlas-CSRF: 1 on POST', async () => {
    const req = await invokeMiddleware('POST');
    expect(req.headers.get('X-Atlas-CSRF')).toBe('1');
  });

  it('does NOT add X-Atlas-CSRF on GET', async () => {
    const req = await invokeMiddleware('GET');
    expect(req.headers.get('X-Atlas-CSRF')).toBeNull();
  });

  it('does NOT add X-Atlas-CSRF on HEAD', async () => {
    const req = await invokeMiddleware('HEAD');
    expect(req.headers.get('X-Atlas-CSRF')).toBeNull();
  });

  it('adds X-Atlas-CSRF on PATCH', async () => {
    const req = await invokeMiddleware('PATCH');
    expect(req.headers.get('X-Atlas-CSRF')).toBe('1');
  });

  it('adds X-Atlas-CSRF on DELETE', async () => {
    const req = await invokeMiddleware('DELETE');
    expect(req.headers.get('X-Atlas-CSRF')).toBe('1');
  });
});

describe('unauthorized response handling', () => {
  async function invokeMiddleware(path: string, status: number): Promise<void> {
    const { unauthorizedMiddlewareForTest } = await import('../api/wrapper');

    await unauthorizedMiddlewareForTest.onResponse?.({
      request: new Request(`http://localhost${path}`),
      response: new Response(null, { status }),
      schemaPath: path,
      params: {},
      id: 'test-id',
      options: {
        baseUrl: 'http://localhost',
        parseAs: 'json',
        querySerializer: () => '',
        bodySerializer: (body: unknown) => JSON.stringify(body),
        pathSerializer: (value: string) => value,
        fetch: globalThis.fetch,
      },
    });
  }

  it('notifies the app when a protected request returns 401', async () => {
    const { setUnauthorizedHandler } = await import('../api/wrapper');
    const handler = vi.fn();
    setUnauthorizedHandler(handler);

    await invokeMiddleware('/api/workspaces/acme/documents/note-a', 401);

    expect(handler).toHaveBeenCalledTimes(1);
  });

  it.each(['/api/auth/me', '/api/auth/login'])('ignores auth endpoint 401s from %s', async (path) => {
    const { setUnauthorizedHandler } = await import('../api/wrapper');
    const handler = vi.fn();
    setUnauthorizedHandler(handler);

    await invokeMiddleware(path, 401);

    expect(handler).not.toHaveBeenCalled();
  });

  it.each([
    '/api/auth/change-password',
    '/api/auth/logout',
  ])('notifies the app when protected auth endpoint %s returns 401', async (path) => {
    const { setUnauthorizedHandler } = await import('../api/wrapper');
    const handler = vi.fn();
    setUnauthorizedHandler(handler);

    await invokeMiddleware(path, 401);

    expect(handler).toHaveBeenCalledTimes(1);
  });

  it('ignores non-401 responses', async () => {
    const { setUnauthorizedHandler } = await import('../api/wrapper');
    const handler = vi.fn();
    setUnauthorizedHandler(handler);

    await invokeMiddleware('/api/workspaces/acme/documents/note-a', 404);

    expect(handler).not.toHaveBeenCalled();
  });

  it('clears an active session and redirects back through login', async () => {
    const { expireSession } = await import('../api/sessionExpiry');
    const auth = { isAuthenticated: true, clearUser: vi.fn() };

    const redirect = expireSession(auth, {
      name: 'notes',
      fullPath: '/n/note-a',
      meta: {},
    });

    expect(auth.clearUser).toHaveBeenCalledTimes(1);
    expect(redirect).toEqual({ name: 'login', query: { redirect: '/n/note-a' } });
  });

  it('does not clear or redirect an already inactive session', async () => {
    const { expireSession } = await import('../api/sessionExpiry');
    const auth = { isAuthenticated: false, clearUser: vi.fn() };

    const redirect = expireSession(auth, {
      name: 'notes',
      fullPath: '/current',
      meta: {},
    });

    expect(auth.clearUser).not.toHaveBeenCalled();
    expect(redirect).toBeNull();
  });

  it.each([
    { name: 'login', meta: {} },
    { name: 'activate', meta: { public: true } },
  ])('clears stale auth without redirecting from $name', async (scenario) => {
    const { expireSession } = await import('../api/sessionExpiry');
    const auth = { isAuthenticated: true, clearUser: vi.fn() };

    const redirect = expireSession(auth, {
      name: scenario.name,
      fullPath: '/current',
      meta: scenario.meta,
    });

    expect(auth.clearUser).toHaveBeenCalledTimes(1);
    expect(redirect).toBeNull();
  });
});

describe('parseProblem', () => {
  it('returns ConflictProblem when type contains revision-conflict', async () => {
    const { parseProblem } = await import('../api/problem');

    const body = {
      type: 'urn:atlas:error:revision-conflict',
      title: 'Revision conflict',
      status: 409,
      detail: 'The document was modified',
      hint: 'Merge your changes and retry',
      request_id: 'req-123',
      current_revision_id: 'rev-456',
      current_seq: 3,
      base_to_current_patch: '@@ -1 +1 @@\n-old\n+new\n',
    };

    const resp = new Response(JSON.stringify(body), {
      status: 409,
      headers: { 'Content-Type': 'application/problem+json' },
    });

    const problem = await parseProblem(resp);

    expect(problem.type).toContain('revision-conflict');
    expect('current_revision_id' in problem).toBe(true);
    if ('current_revision_id' in problem) {
      expect(problem.current_revision_id).toBe('rev-456');
      expect(problem.base_to_current_patch).toBe('@@ -1 +1 @@\n-old\n+new\n');
    }
  });

  it('returns a plain AtlasProblem for generic errors', async () => {
    const { parseProblem } = await import('../api/problem');

    const body = {
      type: 'urn:atlas:error:not-found',
      title: 'Not Found',
      status: 404,
      hint: 'Check the resource identifier',
      request_id: 'req-789',
    };

    const resp = new Response(JSON.stringify(body), {
      status: 404,
      headers: { 'Content-Type': 'application/problem+json' },
    });

    const problem = await parseProblem(resp);

    expect(problem.type).toBe('urn:atlas:error:not-found');
    expect('current_revision_id' in problem).toBe(false);
  });
});
