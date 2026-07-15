import { flushPromises, mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import CommentCard from '@/components/comments/CommentCard.vue';

const MarkdownEditorStub = {
  name: 'MarkdownEditor',
  props: ['body', 'editable', 'reading', 'uploadImage'],
  emits: ['change', 'navigate-wikilink'],
  template: '<div data-markdown :data-reading="String(reading)">{{ body }}</div>',
};

const comment = {
  id: 'comment-1',
  body: 'A linked comment',
  author: { id: 'author-1', type: 'user', display_name: 'Jordan' },
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
};

function mountCard(overrides: Record<string, unknown> = {}) {
  return mount(CommentCard, {
    props: {
      comment,
      canEdit: true,
      canDelete: true,
      onSave: vi.fn().mockResolvedValue(true),
      onDelete: vi.fn().mockResolvedValue(true),
      ...overrides,
    },
    global: { stubs: { MarkdownEditor: MarkdownEditorStub, teleport: true } },
  });
}

describe('CommentCard', () => {
  it('renders comment markdown read-only and exposes its available link navigation metadata', async () => {
    const wrapper = mountCard({
      links: [
        { target: { status: 'available', id: 'task-1', type: 'task', label: 'ATL-1' } },
        { target: { status: 'available', id: 'note-1', type: 'document' } },
        { target: { status: 'available', id: 'attachment-1', type: 'attachment' } },
      ],
    });

    expect(wrapper.get('[data-markdown]').attributes('data-reading')).toBe('true');
    expect(wrapper.get('[data-comment-link="task-1"]').text()).toBe('ATL-1');
    expect(wrapper.get('[data-comment-link="note-1"]').text()).toBe('Note link');
    expect(wrapper.get('[data-comment-link="attachment-1"]').text()).toBe('Attachment link');

    await wrapper.get('[data-comment-link="task-1"]').trigger('click');
    expect(wrapper.emitted('navigate-link')).toEqual([
      [{ status: 'available', id: 'task-1', type: 'task', label: 'ATL-1' }, 'comment-1'],
    ]);
  });

  it('redacts unavailable links and event targets to the exact fallback label', () => {
    const wrapper = mountCard({
      links: [
        {
          target: {
            status: 'unavailable',
            label: 'Recurso no disponible',
            leaked_title: 'Do not render',
          },
        },
      ],
      event: {
        id: 'event-1',
        kind: 'link_removed',
        created_at: '2026-01-01T01:00:00Z',
        target: { status: 'unavailable', label: 'Recurso no disponible', leaked_type: 'task' },
      },
    });

    expect(wrapper.get('[data-comment-link-unavailable]').text()).toBe('Recurso no disponible');
    expect(wrapper.get('[data-comment-event]').text()).toContain('Recurso no disponible');
    expect(wrapper.text()).not.toContain('Do not render');
    expect(wrapper.text()).not.toContain('task');
  });

  it('uses supplied attachment hooks for upload, download, delete, image insertion, progress, and errors', async () => {
    const upload = vi.fn().mockResolvedValue({
      id: 'attachment-2',
      comment_id: 'comment-1',
      file_name: 'new.png',
      content_type: 'image/png',
      size_bytes: 20,
      created_at: '2026-01-01T00:00:00Z',
    });
    const download = vi.fn().mockResolvedValue(new Blob(['file']));
    const remove = vi.fn().mockResolvedValue(true);
    const uploadImage = vi.fn().mockResolvedValue('/attachments/attachment-2');
    const wrapper = mountCard({
      attachments: [
        {
          id: 'attachment-1',
          comment_id: 'comment-1',
          file_name: 'report.pdf',
          content_type: 'application/pdf',
          size_bytes: 12,
          created_at: '2026-01-01T00:00:00Z',
        },
      ],
      canManageAttachments: true,
      attachmentUploading: true,
      attachmentError: 'Upload failed',
      onUploadAttachment: upload,
      onDownloadAttachment: download,
      onDeleteAttachment: remove,
      uploadImage,
    });

    expect(wrapper.get('[role="status"]').text()).toContain('Uploading attachment');
    expect(wrapper.get('[role="alert"]').text()).toBe('Upload failed');

    const input = wrapper.get<HTMLInputElement>('[data-comment-attachment-picker]');
    Object.defineProperty(input.element, 'files', {
      value: [new File(['x'], 'new.png', { type: 'image/png' })],
    });
    await input.trigger('change');
    await flushPromises();
    await wrapper.get('[aria-label="Download report.pdf"]').trigger('click');
    await wrapper.get('[aria-label="Delete report.pdf"]').trigger('click');
    await wrapper.get('[data-test="confirm"]').trigger('click');
    await flushPromises();

    expect(upload).toHaveBeenCalledWith(expect.any(File));
    expect(download).toHaveBeenCalledWith('attachment-1');
    expect(remove).toHaveBeenCalledWith('attachment-1');
  });

  it('preserves author edit/delete permissions and hides attachment mutations without permission', () => {
    const wrapper = mountCard({
      canEdit: false,
      canDelete: true,
      canManageAttachments: false,
      attachments: [
        {
          id: 'attachment-1',
          comment_id: 'comment-1',
          file_name: 'report.pdf',
          content_type: 'application/pdf',
          size_bytes: 12,
          created_at: '2026-01-01T00:00:00Z',
        },
      ],
    });

    expect(wrapper.get('[aria-label="Comment actions"]')).toBeDefined();
    expect(wrapper.get('[aria-label="Download report.pdf"]')).toBeDefined();
    expect(wrapper.find('[data-comment-attachment-picker]').exists()).toBe(false);
    expect(wrapper.find('[aria-label="Delete report.pdf"]').exists()).toBe(false);
  });
});
