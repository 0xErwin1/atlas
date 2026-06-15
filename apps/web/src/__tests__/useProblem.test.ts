import { describe, expect, it } from 'vitest';
import type { AtlasProblem } from '../api/problem';
import { useProblem } from '../composables/useProblem';

describe('useProblem', () => {
  it('returns message from title', () => {
    const problem: AtlasProblem = {
      type: 'urn:atlas:error:not-found',
      title: 'Resource not found',
      status: 404,
      hint: 'Check the resource identifier',
      request_id: 'req-abc',
    };

    const { message } = useProblem(problem);

    expect(message).toBe('Resource not found');
  });

  it('returns hint from problem.hint', () => {
    const problem: AtlasProblem = {
      type: 'urn:atlas:error:not-found',
      title: 'Resource not found',
      status: 404,
      hint: 'Check the resource identifier',
      request_id: 'req-abc',
    };

    const { hint } = useProblem(problem);

    expect(hint).toBe('Check the resource identifier');
  });

  it('returns requestId from problem.request_id', () => {
    const problem: AtlasProblem = {
      type: 'urn:atlas:error:not-found',
      title: 'Resource not found',
      status: 404,
      request_id: 'req-abc',
    };

    const { requestId } = useProblem(problem);

    expect(requestId).toBe('req-abc');
  });

  it('does NOT forward detail', () => {
    const problem: AtlasProblem = {
      type: 'urn:atlas:error:auth-failed',
      title: 'Authentication failed',
      status: 401,
      detail: 'Internal stack trace and sensitive info',
      hint: 'Check your credentials',
      request_id: 'req-xyz',
    };

    const result = useProblem(problem);

    expect(Object.values(result)).not.toContain('Internal stack trace and sensitive info');
    expect('detail' in result).toBe(false);
  });
});
