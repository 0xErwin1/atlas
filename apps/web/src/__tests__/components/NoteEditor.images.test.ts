import { mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { describe, expect, it, vi } from 'vitest';

const { platformFetch } = vi.hoisted(() => ({ platformFetch: vi.fn() }));

vi.mock('@/platform/fetch', () => ({ fetchThroughPlatform: platformFetch }));

import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';
import NoteEditor from '@/components/notas/NoteEditor.vue';

/**
 * Inline note images are stored as `/api/…` attachment URLs, which the webview
 * cannot request itself. The note editor must hand the markdown editor a resolver
 * so those images load through the platform transport.
 */
describe('NoteEditor inline images', () => {
  it('resolves API-hosted image sources through the platform', async () => {
    platformFetch.mockResolvedValue({ ok: true, blob: () => Promise.resolve({} as Blob) } as Response);
    URL.createObjectURL = vi.fn(() => 'blob:1');
    URL.revokeObjectURL = vi.fn();

    const wrapper = mount(NoteEditor, {
      props: { ws: 'ws', slug: 'note', body: 'note' },
      global: { plugins: [createPinia()] },
    });

    const resolve = wrapper.getComponent(MarkdownEditor).props('resolveImageSrc') as (
      url: string,
    ) => Promise<string | null>;

    expect(resolve).toEqual(expect.any(Function));
    await expect(resolve('/api/workspaces/acme/attachments/att-1')).resolves.toBe('blob:1');
  });
});
