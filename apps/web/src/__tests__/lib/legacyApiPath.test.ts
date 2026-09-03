import { describe, expect, it } from 'vitest';
import { toCurrentApiPath } from '@/lib/legacyApiPath';

/**
 * Stored document and comment bodies written before the V2 cutover embed
 * attachment links in the retired `/api/workspaces/...` form. The server keeps
 * no alias for that form, so the web rewrites such links at render time.
 */
describe('toCurrentApiPath', () => {
  it('rewrites a V1 attachment path to its V2 acta form', () => {
    expect(toCurrentApiPath('/api/workspaces/acme/tasks/ATL-1/attachments/a/content')).toBe(
      '/api/v2/acta/workspaces/acme/tasks/ATL-1/attachments/a/content',
    );
  });

  it('keeps the query and fragment of a rewritten path', () => {
    expect(toCurrentApiPath('/api/workspaces/acme/attachments/a/content?download=1#x')).toBe(
      '/api/v2/acta/workspaces/acme/attachments/a/content?download=1#x',
    );
  });

  it('rewrites a same-origin absolute URL whose pathname is a V1 attachment path', () => {
    const origin = globalThis.location.origin;
    const host = globalThis.location.host;

    expect(toCurrentApiPath(`${origin}/api/workspaces/acme/attachments/a/content`)).toBe(
      `${origin}/api/v2/acta/workspaces/acme/attachments/a/content`,
    );
    expect(toCurrentApiPath(`//${host}/api/workspaces/acme/attachments/a/content`)).toBe(
      `//${host}/api/v2/acta/workspaces/acme/attachments/a/content`,
    );
  });

  it('leaves a V1-shaped path on a foreign host untouched', () => {
    expect(toCurrentApiPath('https://atlas.example/api/workspaces/acme/attachments/a/content')).toBe(
      'https://atlas.example/api/workspaces/acme/attachments/a/content',
    );
    expect(toCurrentApiPath('//atlas.example:8080/api/workspaces/acme/attachments/a/content')).toBe(
      '//atlas.example:8080/api/workspaces/acme/attachments/a/content',
    );
  });

  it('leaves an already-V2 path unchanged', () => {
    const current = '/api/v2/acta/workspaces/acme/attachments/a/content';
    expect(toCurrentApiPath(current)).toBe(current);
    expect(toCurrentApiPath(`https://atlas.example${current}`)).toBe(`https://atlas.example${current}`);
  });

  it('leaves external URLs, relative paths, and data URIs unchanged', () => {
    expect(toCurrentApiPath('https://example.com/a.png')).toBe('https://example.com/a.png');
    expect(toCurrentApiPath('mailto:someone@example.com')).toBe('mailto:someone@example.com');
    expect(toCurrentApiPath('api/workspaces/acme/attachments/a/content')).toBe(
      'api/workspaces/acme/attachments/a/content',
    );
    expect(toCurrentApiPath('./api/workspaces/x')).toBe('./api/workspaces/x');
    expect(toCurrentApiPath('#anchor')).toBe('#anchor');
    expect(toCurrentApiPath('data:image/png;base64,AAAA')).toBe('data:image/png;base64,AAAA');
    expect(toCurrentApiPath('')).toBe('');
  });

  it('does not rewrite a path that merely shares the prefix characters', () => {
    expect(toCurrentApiPath('/api/workspaces')).toBe('/api/workspaces');
    expect(toCurrentApiPath('/api/workspacesx/a')).toBe('/api/workspacesx/a');
    expect(toCurrentApiPath('/x/api/workspaces/a')).toBe('/x/api/workspaces/a');
    expect(toCurrentApiPath('https://example.com/x/api/workspaces/a')).toBe(
      'https://example.com/x/api/workspaces/a',
    );
  });
});
