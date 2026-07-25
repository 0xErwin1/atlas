import { mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { describe, expect, it, vi } from 'vitest';

const { platformFetch } = vi.hoisted(() => ({ platformFetch: vi.fn() }));

vi.mock('@/platform/fetch', () => ({ fetchThroughPlatform: platformFetch }));
vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET: vi.fn(), POST: vi.fn(), PATCH: vi.fn(), DELETE: vi.fn() },
}));

import CommentCard from '@/components/comments/CommentCard.vue';
import CommentComposer from '@/components/comments/CommentComposer.vue';
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';

/**
 * Comment attachments are `/api/…` URLs, so an image embedded in a comment needs
 * the same platform-transport resolution as note and task bodies.
 */

const comment = {
  id: 'cm-1',
  body: '![shot](/api/workspaces/acme/tasks/ATL-1/comments/cm-1/attachments/a/content)',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  author: { id: 'u1', type: 'user' as const, display_name: 'Ann' },
};

function resolverOf(wrapper: ReturnType<typeof mount>) {
  return wrapper.getComponent(MarkdownEditor).props('resolveImageSrc') as
    | ((url: string) => Promise<string | null>)
    | undefined;
}

describe('comment inline images', () => {
  it('resolves an image rendered in a posted comment', async () => {
    platformFetch.mockResolvedValue({ ok: true, blob: () => Promise.resolve({} as Blob) } as Response);
    URL.createObjectURL = vi.fn(() => 'blob:1');
    URL.revokeObjectURL = vi.fn();

    const wrapper = mount(CommentCard, {
      props: { comment, ws: 'acme' },
      global: { plugins: [createPinia()], stubs: { RouterLink: true } },
    });

    const resolve = resolverOf(wrapper);
    expect(resolve).toEqual(expect.any(Function));
    await expect(resolve?.('/api/a')).resolves.toBe('blob:1');
  });

  it('resolves an image previewed in the composer draft', async () => {
    platformFetch.mockResolvedValue({ ok: true, blob: () => Promise.resolve({} as Blob) } as Response);
    URL.createObjectURL = vi.fn(() => 'blob:2');
    URL.revokeObjectURL = vi.fn();

    const wrapper = mount(CommentComposer, {
      props: {
        target: { kind: 'task', ws: 'acme', readableId: 'ATL-1' },
        onSubmit: vi.fn().mockResolvedValue(true),
      },
      global: { plugins: [createPinia()] },
    });

    const resolve = resolverOf(wrapper);
    expect(resolve).toEqual(expect.any(Function));
    await expect(resolve?.('/api/a')).resolves.toBe('blob:2');
  });
});
