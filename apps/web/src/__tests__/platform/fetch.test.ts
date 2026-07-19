import { describe, expect, it, vi } from 'vitest';
import { createDesktopFetch } from '@/platform/fetch';

function frameDesktopHttpResponse(
  status: number,
  headers: [string, string][],
  body: Uint8Array,
): ArrayBuffer {
  const metaBytes = new TextEncoder().encode(JSON.stringify({ status, headers }));
  const framed = new Uint8Array(4 + metaBytes.length + body.length);

  new DataView(framed.buffer).setUint32(0, metaBytes.length, true);
  framed.set(metaBytes, 4);
  framed.set(body, 4 + metaBytes.length);

  return framed.buffer;
}

describe('desktop fetch adapter', () => {
  it('routes an openapi-fetch request through Rust and reconstructs the HTTP response', async () => {
    const responseBody = new TextEncoder().encode(
      JSON.stringify({ status: 422, title: 'Invalid input', hint: 'Choose another title' }),
    );
    const invoke = vi.fn().mockResolvedValue(
      frameDesktopHttpResponse(
        422,
        [
          ['content-type', 'application/problem+json'],
          ['x-request-id', 'request-1'],
        ],
        responseBody,
      ),
    );
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
      vi.fn().mockResolvedValue(frameDesktopHttpResponse(204, [], new Uint8Array())),
    );

    const response = await desktopFetch(new Request('tauri://localhost/api/workspaces'));

    expect(response.status).toBe(204);
    expect(await response.text()).toBe('');
  });
});
