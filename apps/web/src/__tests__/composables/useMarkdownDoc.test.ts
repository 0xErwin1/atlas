import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET: vi.fn(),
    PUT: vi.fn(),
  },
}));

import { wrappedClient } from '@/api/wrapper';
import { useMarkdownDoc } from '@/composables/useMarkdownDoc';

const mockGet = wrappedClient.GET as ReturnType<typeof vi.fn>;
const mockPut = wrappedClient.PUT as ReturnType<typeof vi.fn>;

const WS = 'acme';
const SLUG = 'my-doc';

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
});

describe('useMarkdownDoc', () => {
  it('load: returns body and meta after splitting frontmatter', async () => {
    const rawContent = '---\ntitle: My Doc\n---\n\nHello world.';
    mockGet.mockResolvedValue({
      data: {
        slug: SLUG,
        content: rawContent,
        head_revision_id: 'rev-abc',
      },
      error: undefined,
    });

    const { load } = useMarkdownDoc();
    const result = await load(WS, SLUG);

    expect(result.body).toBe('\nHello world.');
    expect(result.meta.title).toBe('My Doc');
    expect(result.headRevisionId).toBe('rev-abc');
  });

  it('load: returns full content as body when no frontmatter', async () => {
    mockGet.mockResolvedValue({
      data: {
        slug: SLUG,
        content: 'No frontmatter here.',
        head_revision_id: 'rev-xyz',
      },
      error: undefined,
    });

    const { load } = useMarkdownDoc();
    const result = await load(WS, SLUG);

    expect(result.body).toBe('No frontmatter here.');
    expect(result.meta).toEqual({});
    expect(result.headRevisionId).toBe('rev-xyz');
  });

  it('save: joins frontmatter+body and calls PUT with base_revision_id', async () => {
    mockPut.mockResolvedValue({ data: {}, error: undefined });

    const { save } = useMarkdownDoc();
    const result = await save(WS, SLUG, '\nBody text.', { title: 'My Doc' }, 'rev-abc');

    expect(mockPut).toHaveBeenCalledWith(
      expect.stringContaining('/documents/{slug}/content'),
      expect.objectContaining({
        body: expect.objectContaining({ base_revision_id: 'rev-abc' }),
      }),
    );

    expect(result.kind).toBe('ok');
  });

  it('save: returns conflict when PUT returns 409', async () => {
    const conflictPayload = {
      type: 'urn:atlas:error:revision-conflict',
      title: 'Revision conflict',
      status: 409,
      current_revision_id: 'rev-new',
      current_seq: 5,
      base_to_current_patch: '@@ -1 +1 @@\n-old\n+new',
    };

    mockPut.mockResolvedValue({
      data: undefined,
      error: conflictPayload,
    });

    const { save } = useMarkdownDoc();
    const result = await save(WS, SLUG, '\nBody.', {}, 'rev-abc');

    expect(result.kind).toBe('conflict');
    if (result.kind === 'conflict') {
      expect(result.problem.current_revision_id).toBe('rev-new');
      expect(result.problem.base_to_current_patch).toBe('@@ -1 +1 @@\n-old\n+new');
    }
  });

  it('save: returns error when PUT fails with non-409 error', async () => {
    mockPut.mockResolvedValue({
      data: undefined,
      error: {
        type: 'urn:atlas:error:not-found',
        title: 'Not Found',
        status: 404,
        hint: 'Document not found',
      },
    });

    const { save } = useMarkdownDoc();
    const result = await save(WS, SLUG, 'Body.', {}, 'rev-abc');

    expect(result.kind).toBe('error');
    if (result.kind === 'error') {
      expect(result.hint).toBe('Document not found');
    }
  });
});
