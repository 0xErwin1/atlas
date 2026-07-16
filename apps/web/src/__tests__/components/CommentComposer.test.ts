import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import CommentComposer from '@/components/comments/CommentComposer.vue';

const { remove, post } = vi.hoisted(() => ({ remove: vi.fn(), post: vi.fn() }));

vi.mock('@/api/wrapper', () => ({ wrappedClient: { DELETE: remove, POST: post } }));

const MarkdownEditorStub = {
  name: 'MarkdownEditor',
  props: ['body', 'uploadImage'],
  emits: ['change'],
  template: '<textarea :value="body" @input="$emit(\'change\', $event.target.value)" />',
};

describe('CommentComposer', () => {
  beforeEach(() => {
    post.mockReset();
    remove.mockReset();
    vi.unstubAllGlobals();
  });

  it('provides draft-backed image uploads and labels the composer controls', () => {
    const wrapper = mount(CommentComposer, {
      props: {
        target: { kind: 'task', ws: 'acme', readableId: 'ATL-1' },
        onSubmit: vi.fn().mockResolvedValue(true),
      },
      global: { stubs: { MarkdownEditor: MarkdownEditorStub } },
    });

    expect(wrapper.getComponent(MarkdownEditorStub).props('uploadImage')).toEqual(expect.any(Function));
    expect(wrapper.get('[data-test="comment-submit"]').attributes('aria-label')).toBe('Post comment');
  });

  it('rejects whitespace-only drafts without changing Markdown submitted to the host', async () => {
    const onSubmit = vi.fn().mockResolvedValue(true);
    const body = '  ```md\ncontent\n```  \n';
    const wrapper = mount(CommentComposer, {
      props: { target: { kind: 'task', ws: 'acme', readableId: 'ATL-1' }, onSubmit },
      global: { stubs: { MarkdownEditor: MarkdownEditorStub } },
    });

    await wrapper.get('textarea').setValue(body);
    await wrapper.get('[data-test="comment-submit"]').trigger('click');

    expect(onSubmit).toHaveBeenCalledWith(body);

    await wrapper.get('textarea').setValue(' \n\t ');
    expect(wrapper.get('[data-test="comment-submit"]').attributes('disabled')).toBeDefined();
  });

  it('creates a draft only after file selection, submits its id, and resets after finalization succeeds', async () => {
    vi.stubGlobal('crypto', { randomUUID: vi.fn().mockReturnValue('00000000-0000-4000-8000-000000000001') });
    post
      .mockResolvedValueOnce({ data: { id: 'draft-1', expires_at: '2026-07-17T00:00:00Z' } })
      .mockResolvedValueOnce({ data: { id: 'file-1', url: '/file', markdown: '[file](/file)' } });
    const onSubmit = vi.fn().mockResolvedValue(true);
    const wrapper = mount(CommentComposer, {
      props: { target: { kind: 'task', ws: 'acme', readableId: 'ATL-1' }, onSubmit },
      global: { stubs: { MarkdownEditor: MarkdownEditorStub } },
    });
    const picker = wrapper.get('input[type="file"]');
    Object.defineProperty(picker.element, 'files', {
      configurable: true,
      value: [new File(['file'], 'file.txt', { type: 'text/plain' })],
    });

    await picker.trigger('change');
    await flushPromises();
    expect(post).toHaveBeenCalled();
    await vi.waitFor(() => expect(wrapper.text()).toContain('Uploaded'));
    await wrapper.get('textarea').setValue('Publish this');
    await wrapper.get('[data-test="comment-submit"]').trigger('click');

    expect(onSubmit).toHaveBeenCalledWith('Publish this', 'draft-1');
    expect(wrapper.find('[aria-label="Comment draft attachments"]').exists()).toBe(false);
  });

  it('retains a finalized draft after a rejected publication and retries with the same identity', async () => {
    vi.stubGlobal('crypto', { randomUUID: vi.fn().mockReturnValue('00000000-0000-4000-8000-000000000001') });
    post
      .mockResolvedValueOnce({ data: { id: 'draft-2', expires_at: '2026-07-17T00:00:00Z' } })
      .mockResolvedValueOnce({ data: { id: 'file-2', url: '/file-2', markdown: '[file](/file-2)' } });
    const onSubmit = vi.fn().mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce(true);
    const wrapper = mount(CommentComposer, {
      props: { target: { kind: 'document', ws: 'acme', slug: 'note' }, onSubmit },
      global: { stubs: { MarkdownEditor: MarkdownEditorStub } },
    });
    const picker = wrapper.get('input[type="file"]');
    Object.defineProperty(picker.element, 'files', {
      configurable: true,
      value: [new File(['file'], 'file.txt', { type: 'text/plain' })],
    });

    await picker.trigger('change');
    await flushPromises();
    await wrapper.get('textarea').setValue('Publish this');
    await wrapper.get('[data-test="comment-submit"]').trigger('click');
    await flushPromises();

    expect(onSubmit).toHaveBeenCalledWith('Publish this', 'draft-2');
    expect((wrapper.get('textarea').element as HTMLTextAreaElement).value).toBe('Publish this');
    expect(wrapper.get('[aria-label="Comment draft attachments"]').text()).toContain('Uploaded');
    expect(wrapper.get('[data-test="comment-submit"]').text()).toContain('Retry');

    await wrapper.get('[data-test="comment-submit"]').trigger('click');
    await flushPromises();

    expect(onSubmit).toHaveBeenNthCalledWith(2, 'Publish this', 'draft-2');
    expect((wrapper.get('textarea').element as HTMLTextAreaElement).value).toBe('');
    expect(wrapper.find('[aria-label="Comment draft attachments"]').exists()).toBe(false);
  });

  it('shows an accessible queued upload, prevents publication, and recovers it with retry', async () => {
    let resolveDraft: ((value: { data: { id: string; expires_at: string } }) => void) | undefined;
    const created = new Promise<{ data: { id: string; expires_at: string } }>((resolve) => {
      resolveDraft = resolve;
    });
    post
      .mockReturnValueOnce(created)
      .mockResolvedValueOnce({ error: { detail: 'offline' } })
      .mockResolvedValueOnce({ data: { id: 'file-3', url: '/file-3', markdown: '[file](/file-3)' } });
    const onSubmit = vi.fn().mockResolvedValue(true);
    const wrapper = mount(CommentComposer, {
      props: { target: { kind: 'task', ws: 'acme', readableId: 'ATL-1' }, onSubmit },
      global: { stubs: { MarkdownEditor: MarkdownEditorStub } },
    });
    const picker = wrapper.get('input[type="file"]');
    Object.defineProperty(picker.element, 'files', {
      configurable: true,
      value: [new File(['file'], 'file.txt', { type: 'text/plain' })],
    });

    await picker.trigger('change');
    await wrapper.get('textarea').setValue('Do not publish yet');
    expect(wrapper.get('[role="status"]').text()).toBe('Queued');
    expect((wrapper.get('[data-test="comment-submit"]').element as HTMLButtonElement).disabled).toBe(true);

    if (resolveDraft === undefined) throw new Error('Draft creation did not start');
    resolveDraft({ data: { id: 'draft-3', expires_at: '2026-07-17T00:00:00Z' } });
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toContain('Failed to upload attachment');

    await wrapper.get('[aria-label="Retry file.txt"]').trigger('click');
    await flushPromises();
    expect(wrapper.get('[aria-label="Comment draft attachments"]').text()).toContain('Uploaded');
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('disables publication while deletion is unresolved and retains the attachment after a finalization race', async () => {
    post
      .mockResolvedValueOnce({ data: { id: 'draft-6', expires_at: '2026-07-17T00:00:00Z' } })
      .mockResolvedValueOnce({ data: { id: 'file-6', url: '/file-6', markdown: '[file](/file-6)' } });
    let resolveDelete: ((value: { error: { status: number; detail: string } }) => void) | undefined;
    remove.mockReturnValue(
      new Promise<{ error: { status: number; detail: string } }>((resolve) => {
        resolveDelete = resolve;
      }),
    );
    const onSubmit = vi.fn().mockResolvedValue(true);
    const wrapper = mount(CommentComposer, {
      props: { target: { kind: 'task', ws: 'acme', readableId: 'ATL-1' }, onSubmit },
      global: { stubs: { MarkdownEditor: MarkdownEditorStub } },
    });
    const picker = wrapper.get('input[type="file"]');
    Object.defineProperty(picker.element, 'files', {
      configurable: true,
      value: [new File(['file'], 'file.txt', { type: 'text/plain' })],
    });

    await picker.trigger('change');
    await flushPromises();
    await wrapper.get('textarea').setValue('Publish this');
    await wrapper.get('[aria-label="Remove file.txt"]').trigger('click');

    expect(wrapper.get('[aria-label="Comment draft attachments"]').text()).toContain('Deleting');
    expect((wrapper.get('[data-test="comment-submit"]').element as HTMLButtonElement).disabled).toBe(true);
    expect(onSubmit).not.toHaveBeenCalled();

    if (resolveDelete === undefined) throw new Error('Delete did not start');
    resolveDelete({ error: { status: 409, detail: 'draft finalized' } });
    await flushPromises();

    expect(wrapper.get('[aria-label="Comment draft attachments"]').text()).toContain('file.txt');
    expect(wrapper.get('[role="alert"]').text()).toContain('Failed to remove attachment');
    expect((wrapper.get('[data-test="comment-submit"]').element as HTMLButtonElement).disabled).toBe(false);
  });

  it('explicitly discards a document draft and resets the draft body and files', async () => {
    post
      .mockResolvedValueOnce({ data: { id: 'draft-4', expires_at: '2026-07-17T00:00:00Z' } })
      .mockResolvedValueOnce({ data: { id: 'file-4', url: '/file-4', markdown: '[file](/file-4)' } });
    remove.mockResolvedValue({});
    const wrapper = mount(CommentComposer, {
      props: {
        target: { kind: 'document', ws: 'acme', slug: 'note' },
        onSubmit: vi.fn().mockResolvedValue(true),
      },
      global: { stubs: { MarkdownEditor: MarkdownEditorStub } },
    });
    const picker = wrapper.get('input[type="file"]');
    Object.defineProperty(picker.element, 'files', {
      configurable: true,
      value: [new File(['file'], 'file.txt', { type: 'text/plain' })],
    });

    await picker.trigger('change');
    await flushPromises();
    await wrapper.get('textarea').setValue('Discard this');
    await wrapper.get('[aria-label="Discard comment draft"]').trigger('click');
    await flushPromises();

    expect(remove).toHaveBeenCalledWith(
      '/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}',
      expect.objectContaining({ params: { path: { ws: 'acme', slug: 'note', draft_id: 'draft-4' } } }),
    );
    expect((wrapper.get('textarea').element as HTMLTextAreaElement).value).toBe('');
    expect(wrapper.find('[aria-label="Comment draft attachments"]').exists()).toBe(false);
  });

  it.each([
    { target: { kind: 'task' as const, ws: 'acme', readableId: 'ATL-1' }, draftId: 'task-draft' },
    { target: { kind: 'document' as const, ws: 'acme', slug: 'note' }, draftId: 'document-draft' },
  ])('never publishes or cancels a $target.kind draft when the composer unmounts', async ({
    target,
    draftId,
  }) => {
    post
      .mockResolvedValueOnce({ data: { id: draftId, expires_at: '2026-07-17T00:00:00Z' } })
      .mockResolvedValueOnce({ data: { id: 'file-5', url: '/file-5', markdown: '[file](/file-5)' } });
    const onSubmit = vi.fn().mockResolvedValue(true);
    const wrapper = mount(CommentComposer, {
      props: { target, onSubmit },
      global: { stubs: { MarkdownEditor: MarkdownEditorStub } },
    });
    const picker = wrapper.get('input[type="file"]');
    Object.defineProperty(picker.element, 'files', {
      configurable: true,
      value: [new File(['file'], 'file.txt', { type: 'text/plain' })],
    });

    await picker.trigger('change');
    await flushPromises();
    wrapper.unmount();

    expect(onSubmit).not.toHaveBeenCalled();
    expect(remove).not.toHaveBeenCalled();
  });
});
