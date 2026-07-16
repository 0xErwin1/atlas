export interface DesktopHttpRequest {
  method: string;
  path: string;
  headers: [string, string][];
  body: number[];
}

export interface DesktopHttpResponse {
  status: number;
  headers: [string, string][];
  body: number[];
}

export type DesktopInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

type PlatformFetch = (request: Request) => Promise<Response>;

let platformFetch: PlatformFetch = (request) => globalThis.fetch(request);

export function setPlatformFetch(fetch: PlatformFetch): void {
  platformFetch = fetch;
}

export function resetPlatformFetchForTest(): void {
  platformFetch = (request) => globalThis.fetch(request);
}

export function fetchThroughPlatform(request: Request): Promise<Response> {
  return platformFetch(request);
}

export function createDesktopFetch(invoke: DesktopInvoke): PlatformFetch {
  return async (request) => {
    const url = new URL(request.url);
    const body =
      request.method === 'GET' || request.method === 'HEAD' ? [] : await request.clone().arrayBuffer();
    const response = await invoke<DesktopHttpResponse>('desktop_api_request', {
      request: {
        method: request.method,
        path: `${url.pathname}${url.search}`,
        headers: Array.from(request.headers.entries()),
        body: Array.from(new Uint8Array(body)),
      } satisfies DesktopHttpRequest,
    });
    const bodyless = response.status === 204 || response.status === 205 || response.status === 304;

    return new Response(bodyless ? undefined : new Uint8Array(response.body), {
      status: response.status,
      headers: response.headers,
    });
  };
}
