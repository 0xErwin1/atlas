export interface DesktopHttpRequest {
  method: string;
  path: string;
  headers: [string, string][];
  body: number[];
}

export interface DesktopHttpResponseMeta {
  status: number;
  headers: [string, string][];
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
    const framed = await invoke<ArrayBuffer>('desktop_api_request', {
      request: {
        method: request.method,
        path: `${url.pathname}${url.search}`,
        headers: Array.from(request.headers.entries()),
        body: Array.from(new Uint8Array(body)),
      } satisfies DesktopHttpRequest,
    });

    const { meta, body: bodyBytes } = decodeDesktopHttpResponse(framed);
    const bodyless = meta.status === 204 || meta.status === 205 || meta.status === 304;

    return new Response(bodyless ? undefined : bodyBytes, {
      status: meta.status,
      headers: meta.headers,
    });
  };
}

// Decodes the framed `desktop_api_request` response: a `u32` little-endian
// length prefix, that many bytes of meta JSON, then the raw body bytes. The
// endianness and prefix width match the Rust host exactly (`u32::to_le_bytes`).
function decodeDesktopHttpResponse(framed: ArrayBuffer): {
  meta: DesktopHttpResponseMeta;
  body: Uint8Array<ArrayBuffer>;
} {
  const view = new DataView(framed);
  const metaLength = view.getUint32(0, true);

  const metaBytes = new Uint8Array(framed, 4, metaLength);
  const meta = JSON.parse(new TextDecoder().decode(metaBytes)) as DesktopHttpResponseMeta;

  const body = new Uint8Array(framed, 4 + metaLength);

  return { meta, body };
}
