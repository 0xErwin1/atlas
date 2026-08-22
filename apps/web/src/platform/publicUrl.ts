import { getPlatformTransport } from './transport';

export type OpenPublicPathOutcome = 'opened' | 'unknown-base' | 'failed';

/**
 * Builds an absolute, shareable URL for `path` against the active platform's
 * public base. An already-absolute `http(s)` path is returned unchanged.
 * Returns `null` when the base is not known yet (desktop host has not
 * answered), so callers must not fall back to a relative URL.
 */
export function toPublicUrl(path: string): string | null {
  if (/^https?:\/\//i.test(path)) return path;

  const base = getPlatformTransport().publicBase();
  if (base === '') return null;

  return `${base}${path.startsWith('/') ? '' : '/'}${path}`;
}

/**
 * Resolves `path` to a public URL and opens it through the active platform
 * transport. Never throws: a missing base or a transport failure resolve to
 * `'unknown-base'` or `'failed'` instead of a rejection.
 */
export async function openPublicPath(path: string): Promise<OpenPublicPathOutcome> {
  const url = toPublicUrl(path);
  if (url === null) return 'unknown-base';

  try {
    const result = await getPlatformTransport().openExternal(url);
    return result.error != null ? 'failed' : 'opened';
  } catch {
    return 'failed';
  }
}
