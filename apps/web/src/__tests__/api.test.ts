import { describe, expect, it } from 'vitest';

describe('CSRF wrapper', () => {
  async function invokeMiddleware(method: string): Promise<Request> {
    const { csrfMiddlewareForTest } = await import('../api/wrapper');

    const original = new Request(`http://localhost/v1/test`, { method });

    const result = await csrfMiddlewareForTest.onRequest?.({
      request: original,
      schemaPath: '/v1/test',
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
