import { describe, expect, it, vi } from 'vitest';
import { createDesktopFetch } from '@/platform/fetch';

describe('desktop fetch adapter', () => {
  it('routes an openapi-fetch request through Rust and reconstructs the HTTP response', async () => {
    const invoke = vi.fn().mockResolvedValue({
      status: 422,
      headers: [
        ['content-type', 'application/problem+json'],
        ['x-request-id', 'request-1'],
      ],
      body: Array.from(
        new TextEncoder().encode(
          JSON.stringify({ status: 422, title: 'Invalid input', hint: 'Choose another title' }),
        ),
      ),
    });
    const desktopFetch = createDesktopFetch(invoke);
    const request = new Request('tauri://localhost/api/workspaces/acme/documents?cursor=next', {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-atlas-csrf': '1' },
      body: JSON.stringify({ title: 'Draft' }),
    });

    const response = await desktopFetch(request);

    expect(invoke).toHaveBeenCalledWith('desktop_api_request', {
      request: {
        method: 'POST',
        path: '/api/workspaces/acme/documents?cursor=next',
        headers: expect.arrayContaining([
          ['content-type', 'application/json'],
          ['x-atlas-csrf', '1'],
        ]),
        body: Array.from(new TextEncoder().encode(JSON.stringify({ title: 'Draft' }))),
      },
    });
    expect(response.status).toBe(422);
    expect(response.headers.get('content-type')).toBe('application/problem+json');
    expect(await response.json()).toEqual({
      status: 422,
      title: 'Invalid input',
      hint: 'Choose another title',
    });
  });

  it('omits an empty body for bodyless response statuses', async () => {
    const desktopFetch = createDesktopFetch(
      vi.fn().mockResolvedValue({ status: 204, headers: [], body: [] }),
    );

    const response = await desktopFetch(new Request('tauri://localhost/api/workspaces'));

    expect(response.status).toBe(204);
    expect(await response.text()).toBe('');
  });
});
