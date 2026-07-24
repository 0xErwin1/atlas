import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET: vi.fn().mockResolvedValue({ data: undefined }),
    POST: vi.fn(),
    PATCH: vi.fn(),
    DELETE: vi.fn(),
  },
}));

import AttachmentList from '@/components/tareas/AttachmentList.vue';

const actor = { id: 'user-1', type: 'user' as const, display_name: 'Jordan' };

function attachment(overrides: Record<string, unknown> = {}) {
  return {
    id: 'att-1',
    file_name: 'report.pdf',
    content_type: 'application/pdf',
    size_bytes: 10,
    created_by: actor,
    created_at: '2026-01-01T00:00:00Z',
    ...overrides,
  };
}

function list(attachments: ReturnType<typeof attachment>[]) {
  return mount(AttachmentList, {
    props: { attachments, ws: 'acme', readableId: 'ATL-1' },
    global: { plugins: [createPinia()] },
  });
}

describe('AttachmentList description references', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    URL.createObjectURL = vi.fn(() => 'blob:1');
    URL.revokeObjectURL = vi.fn();
  });

  it('references a non-image attachment as a link to its content', async () => {
    const wrapper = list([attachment()]);

    await wrapper.get('[aria-label="Reference report.pdf in the description"]').trigger('click');

    expect(wrapper.emitted('insert')?.[0]).toEqual([
      '[report.pdf](/api/workspaces/acme/tasks/ATL-1/attachments/att-1/content)',
    ]);
  });

  it('references an image as an embed so it renders inline', async () => {
    const wrapper = list([attachment({ id: 'att-2', file_name: 'diagram.png', content_type: 'image/png' })]);

    await wrapper.get('[aria-label="Reference diagram.png in the description"]').trigger('click');

    expect(wrapper.emitted('insert')?.[0]).toEqual([
      '![diagram](/api/workspaces/acme/tasks/ATL-1/attachments/att-2/content)',
    ]);
  });

  it('offers the reference action for every attachment', () => {
    const wrapper = list([attachment(), attachment({ id: 'att-2', file_name: 'b.txt' })]);

    expect(wrapper.findAll('[data-test="attachment-reference"]')).toHaveLength(2);
  });
});
