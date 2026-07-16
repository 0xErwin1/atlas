import { flushPromises, mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { describe, expect, it, vi } from 'vitest';
import { type ImageUploadResult, imageUploadInsertion } from '@/components/editor/imageUpload';
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';

describe('MarkdownEditor image uploads', () => {
  it('preserves legacy string URLs with the existing image Markdown syntax', () => {
    const result: ImageUploadResult = '/attachments/legacy';

    expect(imageUploadInsertion(result, 'architecture.diagram.png', false)).toBe(
      '\n![architecture.diagram](/attachments/legacy)\n',
    );
  });

  it('uses a structured result URL with the existing sanitized image alt text', () => {
    const result: ImageUploadResult = { url: '/attachments/structured' };

    expect(imageUploadInsertion(result, ' ]diagram\n.png', true)).toBe(
      '![diagram](/attachments/structured)\n',
    );
  });

  it('inserts explicit Markdown from a structured result verbatim', () => {
    const result: ImageUploadResult = {
      url: '/attachments/structured',
      markdown: '[diagram download](/attachments/structured)',
    };

    expect(imageUploadInsertion(result, 'ignored.png', false)).toBe(
      '[diagram download](/attachments/structured)',
    );
  });

  it('does not insert Markdown when an upload is cancelled or fails', () => {
    const cancelled: ImageUploadResult = null;

    expect(imageUploadInsertion(cancelled, 'cancelled.png', false)).toBeNull();
  });

  it('does not insert Markdown when the image upload callback rejects', async () => {
    const uploadImage = vi.fn().mockRejectedValue(new Error('Upload failed'));
    const wrapper = mount(MarkdownEditor, {
      props: { body: 'Draft', embeddedControls: false, uploadImage },
      global: { plugins: [createPinia()] },
    });
    const file = new File(['image'], 'failure.png', { type: 'image/png' });
    const event = new Event('paste', { bubbles: true, cancelable: true });
    Object.defineProperty(event, 'clipboardData', {
      value: { items: [{ kind: 'file', getAsFile: () => file }] },
    });

    wrapper.get('.cm-content').element.dispatchEvent(event);
    await flushPromises();

    expect(uploadImage).toHaveBeenCalledWith(file);
    expect(wrapper.emitted('change')).toBeUndefined();
  });
});
