import { onScopeDispose } from 'vue';
import { toCurrentApiPath } from '@/lib/legacyApiPath';
import { fetchThroughPlatform } from '@/platform/fetch';

const API_PREFIX = '/api/';

/**
 * Resolves Markdown image sources that point at the Atlas API into object URLs
 * fetched through the platform transport.
 *
 * A native `<img src="/api/…">` is issued by the webview itself, so on desktop it
 * hits the app's asset origin — which serves no API — and never carries the
 * session. Routing the bytes through `fetchThroughPlatform` keeps the request on
 * the same path as every other API call (IPC bridge on desktop, credentialed
 * fetch in the browser).
 *
 * Sources that are not API paths are returned unchanged so external images keep
 * loading directly. A V1 attachment path from pre-cutover stored content is
 * fetched from its V2 form (see `toCurrentApiPath`). Each source is fetched once
 * and shared; object URLs are revoked when the owning scope is disposed, which is
 * why the resolver — not its callers — owns their lifetime.
 */
export function useApiImageSrc(): (url: string) => Promise<string | null> {
  const resolved = new Map<string, Promise<string | null>>();
  const objectUrls: string[] = [];

  async function load(url: string): Promise<string | null> {
    const response = await fetchThroughPlatform(new Request(new URL(url, globalThis.location.href)));
    if (!response.ok) return null;

    const objectUrl = URL.createObjectURL(await response.blob());
    objectUrls.push(objectUrl);
    return objectUrl;
  }

  onScopeDispose(() => {
    for (const objectUrl of objectUrls) URL.revokeObjectURL(objectUrl);
    objectUrls.length = 0;
    resolved.clear();
  });

  return (source: string): Promise<string | null> => {
    const url = toCurrentApiPath(source);
    if (!url.startsWith(API_PREFIX)) return Promise.resolve(url);

    const cached = resolved.get(url);
    if (cached !== undefined) return cached;

    // A failed load is forgotten rather than cached, so a transient error does not
    // leave the image permanently blank for the lifetime of the view.
    const request = load(url)
      .catch(() => null)
      .then((src) => {
        if (src === null) resolved.delete(url);
        return src;
      });

    resolved.set(url, request);
    return request;
  };
}
