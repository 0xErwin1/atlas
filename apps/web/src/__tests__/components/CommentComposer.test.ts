import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import CommentComposer from '@/components/comments/CommentComposer.vue';

const MarkdownEditorStub = {
  name: 'MarkdownEditor',
  props: ['body', 'uploadImage'],
  emits: ['change'],
  template: '<textarea :value="body" @input="$emit(\'change\', $event.target.value)" />',
};

describe('CommentComposer', () => {
  it('is text-only by construction and labels the composer controls', () => {
    const wrapper = mount(CommentComposer, {
      props: { onSubmit: vi.fn().mockResolvedValue(true) },
      global: { stubs: { MarkdownEditor: MarkdownEditorStub } },
    });

    expect(wrapper.getComponent(MarkdownEditorStub).props('uploadImage')).toBeUndefined();
    expect(wrapper.get('[data-test="comment-submit"]').attributes('aria-label')).toBe('Post comment');
  });

  it('rejects whitespace-only drafts without changing Markdown submitted to the host', async () => {
    const onSubmit = vi.fn().mockResolvedValue(true);
    const body = '  ```md\ncontent\n```  \n';
    const wrapper = mount(CommentComposer, {
      props: { onSubmit },
      global: { stubs: { MarkdownEditor: MarkdownEditorStub } },
    });

    await wrapper.get('textarea').setValue(body);
    await wrapper.get('[data-test="comment-submit"]').trigger('click');

    expect(onSubmit).toHaveBeenCalledWith(body);

    await wrapper.get('textarea').setValue(' \n\t ');
    expect(wrapper.get('[data-test="comment-submit"]').attributes('disabled')).toBeDefined();
  });
});
