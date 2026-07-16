import { type DOMWrapper, flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick, ref } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import CommentCard from '@/components/comments/CommentCard.vue';
import DocumentComments from '@/components/notas/DocumentComments.vue';
import { useAuthStore } from '@/stores/auth';
import { type CommentDto, useDocumentsStore } from '@/stores/documents';
import { useWorkspaceStore } from '@/stores/workspace';

const { draftDelete, draftPost } = vi.hoisted(() => ({ draftDelete: vi.fn(), draftPost: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { DELETE: draftDelete, POST: draftPost },
}));

const commentFeed = {
  entries: ref<unknown[]>([]),
  hasMore: ref(false),
  status: ref<'idle' | 'pending' | 'ready' | 'error'>('ready'),
  error: ref<string | null>(null),
  load: vi.fn().mockResolvedValue(undefined),
  loadMore: vi.fn().mockResolvedValue(undefined),
};

const commentAttachments = {
  items: ref<Record<string, unknown[]>>({}),
  error: ref<Record<string, string>>({}),
  isListing: () => false,
  isUploading: () => false,
  isDownloading: () => false,
  isDeleting: () => false,
  reload: vi.fn().mockResolvedValue(undefined),
  upload: vi.fn().mockResolvedValue(null),
  download: vi.fn().mockResolvedValue(undefined),
  delete: vi.fn().mockResolvedValue(true),
  contentUrl: vi.fn().mockReturnValue('/attachment'),
};

vi.mock('@/composables/useCommentFeed', () => ({
  useCommentFeed: () => commentFeed,
}));

vi.mock('@/composables/useCommentAttachments', () => ({
  useCommentAttachments: vi.fn(() => commentAttachments),
}));

const editorFocus = vi.fn();
const RouteStub = { template: '<div />' };
const testRouter = createRouter({
  history: createMemoryHistory(),
  routes: [
    { path: '/t/task/:readableId', name: 'task-detail', component: RouteStub },
    { path: '/n/:slug?', name: 'notes', component: RouteStub },
    { path: '/:pathMatch(.*)*', component: RouteStub },
  ],
});

const MarkdownEditorStub = {
  name: 'MarkdownEditor',
  props: ['body', 'editable', 'reading', 'embeddedControls', 'placeholder', 'minHeight', 'widthToggle'],
  emits: ['change'],
  setup(_props: unknown, { expose }: { expose: (exposed: Record<string, unknown>) => void }) {
    expose({ focus: editorFocus });
  },
  template:
    '<div class="md-stub" :data-editable="String(editable)">' +
    '<span class="md-body">{{ body }}</span>' +
    '<textarea class="md-input" :value="body" @input="$emit(\'change\', $event.target.value)" />' +
    '</div>',
};

const comment = (
  id: string,
  body: string,
  authorId: string,
  authorType = 'user',
  name: string | null = 'Jordan',
  createdAt = '2026-01-01T00:00:00Z',
  updatedAt = createdAt,
): CommentDto => ({
  id,
  document_id: 'd1',
  body,
  author: { id: authorId, type: authorType, display_name: name },
  created_at: createdAt,
  updated_at: updatedAt,
});

function setup(comments: CommentDto[], commentsHasMore = false) {
  const store = useDocumentsStore();
  vi.spyOn(store, 'loadComments').mockResolvedValue();
  store.$patch({ comments, commentsHasMore });
  commentFeed.hasMore.value = commentsHasMore;
  commentFeed.entries.value = comments.map((loadedComment) => ({
    type: 'comment',
    comment: loadedComment,
    links: [],
  }));
  return store;
}

function mountPanel() {
  return mount(DocumentComments, {
    props: { ws: 'acme', slug: 'my-doc' },
    global: { plugins: [testRouter], stubs: { MarkdownEditor: MarkdownEditorStub, teleport: true } },
  });
}

function mountPanelWithRealEditor() {
  return mount(DocumentComments, {
    props: { ws: 'acme', slug: 'my-doc' },
    global: { plugins: [testRouter], stubs: { CommentComposer: true, teleport: true } },
  });
}

function mountPanelWithRealEditors() {
  return mount(DocumentComments, {
    props: { ws: 'acme', slug: 'my-doc' },
    global: { plugins: [testRouter], stubs: { teleport: true } },
  });
}

function droppedImage(file: File): DataTransfer {
  return { files: [file], items: [], getData: () => '' } as unknown as DataTransfer;
}

function deferred<T>() {
  let resolve: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });

  return { promise, resolve: (value: T) => resolve(value) };
}

function enableCodeMirrorDropCoordinates(): void {
  Object.defineProperty(Range.prototype, 'getClientRects', {
    configurable: true,
    value: () => [],
  });
}

function menuItem(wrapper: VueWrapper, label: string): DOMWrapper<Element> | undefined {
  return wrapper.findAll('[role="menuitem"]').find((node) => node.text().trim() === label);
}

function signInAs(id: string, role: 'member' | 'admin' = 'member'): void {
  const auth = useAuthStore();
  auth.user = {
    id,
    username: id,
    display_name: id,
    email: null,
    principal_type: 'user',
    is_root: false,
    is_system_admin: false,
  };
  useWorkspaceStore().members = [
    { id, display: id, principal_type: 'user', role, account_status: 'active', key_type: null },
  ];
}

beforeEach(() => {
  setActivePinia(createPinia());
  editorFocus.mockClear();
  commentFeed.entries.value = [];
  commentFeed.hasMore.value = false;
  commentFeed.status.value = 'ready';
  commentFeed.error.value = null;
  commentAttachments.items.value = {};
  commentAttachments.error.value = {};
  draftDelete.mockReset();
  draftPost.mockReset();
  vi.clearAllMocks();
});

describe('DocumentComments (ATL-37)', () => {
  it('loads and renders the shared document feed with retained events and attachment lifecycle callbacks', async () => {
    const store = setup([]);
    commentFeed.entries.value = [
      {
        type: 'comment',
        comment: comment('c1', 'Attached note', 'me', 'user', 'Me'),
        links: [{ target: { status: 'available', id: 'attachment-1', type: 'attachment' } }],
      },
      {
        type: 'event',
        id: 'e1',
        kind: 'link_removed',
        created_at: '2026-01-01T01:00:00Z',
        target: { status: 'unavailable', label: 'Recurso no disponible' },
      },
    ];
    commentAttachments.items.value = {
      c1: [
        {
          id: 'attachment-1',
          comment_id: 'c1',
          content_type: 'image/png',
          created_at: '2026-01-01T00:00:00Z',
          file_name: 'image.png',
          size_bytes: 12,
        },
      ],
    };
    signInAs('me');

    const wrapper = mountPanel();
    await flushPromises();

    expect(store.loadComments).not.toHaveBeenCalled();
    expect(commentFeed.load).toHaveBeenCalledWith({ kind: 'document', ws: 'acme', slug: 'my-doc' });
    expect(wrapper.get('[data-comment-event="e1"]').text()).toContain('Recurso no disponible');
    expect(wrapper.get('[aria-label="Download image.png"]')).toBeDefined();

    await wrapper.get('[data-comment-link="attachment-1"]').trigger('click');
    await flushPromises();
    expect(commentAttachments.download).toHaveBeenCalledWith('c1', 'attachment-1');
  });

  it('shows the shared feed error and retries it without falling back to the legacy document collection', async () => {
    const store = setup([]);
    commentFeed.status.value = 'error';
    commentFeed.error.value = 'Comment feed denied';

    const wrapper = mountPanel();
    await wrapper.get('[data-state="error"] button').trigger('click');

    expect(wrapper.text()).toContain('Comment feed denied');
    expect(commentFeed.load).toHaveBeenCalledWith({ kind: 'document', ws: 'acme', slug: 'my-doc' });
    expect(store.loadComments).not.toHaveBeenCalled();
  });
  it('loads the shared feed for the open document on mount', () => {
    const store = setup([]);

    mountPanel();

    expect(store.loadComments).not.toHaveBeenCalled();
    expect(commentFeed.load).toHaveBeenCalledWith({ kind: 'document', ws: 'acme', slug: 'my-doc' });
  });

  it('shows a compact empty state when there are no comments', () => {
    setup([]);

    const wrapper = mountPanel();

    expect(wrapper.find('[data-comment-id]').exists()).toBe(false);
    expect(wrapper.find('[data-state="empty"]').exists()).toBe(true);
    expect(wrapper.text()).toContain('No comments yet');
  });

  it('renders a card per comment in store order', () => {
    setup([
      comment('c1', 'First note', 'u1', 'user', 'Jordan'),
      comment('c2', 'Second note', 'k1', 'api_key', 'Claude'),
    ]);

    const wrapper = mountPanel();

    const cards = wrapper.findAll('[data-comment-id]');
    expect(cards.map((c) => c.attributes('data-comment-id'))).toEqual(['c1', 'c2']);
    expect(cards[0]?.text()).toContain('First note');
    expect(cards[1]?.text()).toContain('Claude');
  });

  it('submits the typed body via addComment and clears the composer', async () => {
    const store = setup([]);
    const addComment = vi.spyOn(store, 'addComment').mockResolvedValue(true);

    const wrapper = mountPanel();

    const input = wrapper.get('[data-comment-composer] textarea');
    await input.setValue('New comment');
    await wrapper.get('[data-test="comment-submit"]').trigger('click');
    await flushPromises();

    expect(addComment).toHaveBeenCalledWith('acme', 'my-doc', 'New comment');
    expect((input.element as HTMLTextAreaElement).value).toBe('');
  });

  it('retains a failed document publication verbatim, exposes retry, and creates only one comment after retry', async () => {
    const store = setup([]);
    const addComment = vi.spyOn(store, 'addComment').mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    const body = '  exact **document** Markdown\n\nwith whitespace  ';

    const wrapper = mountPanel();
    const input = wrapper.get('[data-comment-composer] textarea');
    commentFeed.load.mockClear();
    await input.setValue(body);
    await wrapper.get('[data-test="comment-submit"]').trigger('click');
    await flushPromises();

    expect((input.element as HTMLTextAreaElement).value).toBe(body);
    expect(wrapper.get('[data-comment-composer] [role="alert"]').text()).toContain('try again');
    expect(wrapper.get('[data-test="comment-submit"]').text()).toContain('Retry');
    expect(commentAttachments.upload).not.toHaveBeenCalled();
    expect(commentFeed.load).not.toHaveBeenCalledWith({ kind: 'document', ws: 'acme', slug: 'my-doc' });

    await wrapper.get('[data-test="comment-submit"]').trigger('click');
    await flushPromises();

    expect(addComment).toHaveBeenNthCalledWith(1, 'acme', 'my-doc', body);
    expect(addComment).toHaveBeenNthCalledWith(2, 'acme', 'my-doc', body);
    expect((input.element as HTMLTextAreaElement).value).toBe('');
    expect(commentAttachments.upload).not.toHaveBeenCalled();
  });

  it('offers the actions menu to the comment author', async () => {
    setup([comment('c1', 'Mine', 'me', 'user', 'Me')]);
    signInAs('me');

    const wrapper = mountPanel();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');

    expect(menuItem(wrapper, 'Edit')).toBeDefined();
    expect(menuItem(wrapper, 'Delete')).toBeDefined();
  });

  it('hides the actions menu for a member who neither authored nor can moderate', () => {
    setup([comment('c1', 'Theirs', 'someone-else')]);
    signInAs('me');

    const wrapper = mountPanel();

    expect(wrapper.find('[data-comment-id="c1"] [aria-label="Comment actions"]').exists()).toBe(false);
  });

  it('does not provide image uploads for a comment the current actor cannot edit', () => {
    setup([comment('c1', 'Theirs', 'someone-else')]);
    signInAs('me');

    const wrapper = mountPanel();

    expect(wrapper.getComponent(CommentCard).props('uploadImage')).toBeUndefined();
  });

  it('creates one document draft and uploads an image dropped into the real composer without publishing', async () => {
    const store = setup([]);
    const addComment = vi.spyOn(store, 'addComment').mockResolvedValue(true);
    draftPost.mockImplementation((path: string) => {
      if (path.endsWith('/comment-drafts')) {
        return Promise.resolve({ data: { id: 'document-draft', expires_at: '2026-07-17T00:00:00Z' } });
      }

      return Promise.resolve({
        data: { id: 'document-image', url: '/document-image', markdown: '![diagram](/document-image)' },
      });
    });
    const wrapper = mountPanelWithRealEditors();
    const content = wrapper.get('[data-comment-composer] .cm-content');
    enableCodeMirrorDropCoordinates();

    const image = new File(['image'], 'diagram.png', { type: 'image/png' });
    await content.trigger('drop', {
      clientX: 0,
      clientY: 0,
      dataTransfer: droppedImage(image),
    });
    await flushPromises();

    expect(draftPost).toHaveBeenCalledTimes(2);
    expect(draftPost).toHaveBeenNthCalledWith(
      1,
      '/api/workspaces/{ws}/documents/{slug}/comment-drafts',
      expect.objectContaining({ params: expect.objectContaining({ path: { ws: 'acme', slug: 'my-doc' } }) }),
    );
    expect(draftPost).toHaveBeenNthCalledWith(
      2,
      '/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments',
      expect.objectContaining({
        body: [105, 109, 97, 103, 101],
        headers: { 'Content-Type': 'image/png' },
        params: expect.objectContaining({
          path: { ws: 'acme', slug: 'my-doc', draft_id: 'document-draft' },
          header: expect.objectContaining({ 'x-file-name': 'diagram.png' }),
        }),
      }),
    );
    expect(wrapper.get('[data-comment-composer]').text()).toContain('Uploaded');
    expect(commentAttachments.upload).not.toHaveBeenCalled();
    expect(addComment).not.toHaveBeenCalled();
  });

  it('does not upload or alter a document comment when a non-author drops an image into its real read-only editor', async () => {
    setup([comment('c1', 'Original', 'someone-else')]);
    signInAs('me');
    const wrapper = mountPanelWithRealEditors();

    await wrapper.get('[data-comment-id="c1"] .cm-content').trigger('drop', {
      clientX: 0,
      clientY: 0,
      dataTransfer: droppedImage(new File(['image'], 'diagram.png', { type: 'image/png' })),
    });
    await flushPromises();

    expect(commentAttachments.upload).not.toHaveBeenCalled();
    expect(wrapper.get('[data-comment-id="c1"] .cm-content').text()).toBe('Original');
  });

  it('ignores a document image upload that completes after edit permission is revoked', async () => {
    setup([comment('c1', 'Original', 'me', 'user', 'Me')]);
    const pendingUpload = deferred<{ id: string } | null>();
    commentAttachments.upload.mockReturnValue(pendingUpload.promise);
    commentAttachments.contentUrl.mockReturnValue('/document-image');
    signInAs('me');
    enableCodeMirrorDropCoordinates();

    const wrapper = mountPanelWithRealEditor();
    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');
    await menuItem(wrapper, 'Edit')?.trigger('click');
    await wrapper.get('[data-comment-id="c1"] .cm-content').trigger('drop', {
      clientX: 0,
      clientY: 0,
      dataTransfer: droppedImage(new File(['image'], 'diagram.png', { type: 'image/png' })),
    });
    await flushPromises();

    commentFeed.entries.value = [
      { type: 'comment', comment: comment('c2', 'Replacement', 'other', 'user', 'Other'), links: [] },
    ];
    await wrapper.setProps({ ws: 'next', slug: 'next-doc' });
    signInAs('other');
    await nextTick();
    pendingUpload.resolve({ id: 'image-1' });
    await flushPromises();

    expect(commentAttachments.upload).toHaveBeenCalledTimes(1);
    expect(wrapper.find('[data-comment-id="c1"]').exists()).toBe(false);
    expect(wrapper.get('[data-comment-id="c2"] .cm-content').text()).toBe('Replacement');
  });

  it('derives document image Markdown URLs from the shared uploaded attachment ID', async () => {
    setup([comment('c1', 'Mine', 'me', 'user', 'Me')]);
    commentAttachments.upload.mockResolvedValue({ id: 'image-1' });
    commentAttachments.contentUrl.mockReturnValue(
      '/api/workspaces/acme/documents/my-doc/comments/c1/attachments/image-1',
    );
    signInAs('me');

    const wrapper = mountPanel();
    const uploadImage = wrapper.getComponent(CommentCard).props('uploadImage') as
      | ((file: File) => Promise<string | null>)
      | undefined;
    const file = new File(['image'], 'diagram.png', { type: 'image/png' });

    expect(await uploadImage?.(file)).toBe(
      '/api/workspaces/acme/documents/my-doc/comments/c1/attachments/image-1',
    );
    expect(commentAttachments.upload).toHaveBeenCalledWith('c1', file);
    expect(commentAttachments.contentUrl).toHaveBeenCalledWith('c1', 'image-1');
  });

  it('drops an image through the real document comment editor, retains a failed save, and announces retry success', async () => {
    const store = setup([comment('c1', 'Original', 'me', 'user', 'Me')]);
    const editComment = vi
      .spyOn(store, 'editComment')
      .mockResolvedValueOnce(false)
      .mockResolvedValueOnce(true);
    const image = new File(['image'], 'diagram.png', { type: 'image/png' });
    commentAttachments.upload.mockResolvedValue({ id: 'image-1' });
    commentAttachments.contentUrl.mockReturnValue(
      '/api/workspaces/acme/documents/my-doc/comments/c1/attachments/image-1',
    );
    signInAs('me');
    enableCodeMirrorDropCoordinates();

    const wrapper = mountPanelWithRealEditor();
    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');
    await menuItem(wrapper, 'Edit')?.trigger('click');
    await wrapper.get('[data-comment-id="c1"] .cm-content').trigger('drop', {
      clientX: 0,
      clientY: 0,
      dataTransfer: droppedImage(image),
    });
    await flushPromises();
    await wrapper.get('[data-comment-id="c1"] [data-test="comment-edit-save"]').trigger('click');
    await flushPromises();
    expect(wrapper.get('[data-comment-id="c1"] [role="alert"]').text()).toContain('Could not save comment');
    expect(wrapper.find('[data-comment-attachment-announcement]').exists()).toBe(false);
    await wrapper.get('[data-comment-id="c1"] [data-test="comment-edit-save"]').trigger('click');
    await flushPromises();

    expect(commentAttachments.upload).toHaveBeenCalledWith('c1', image);
    expect(editComment).toHaveBeenCalledWith(
      'acme',
      'my-doc',
      'c1',
      '![diagram](/api/workspaces/acme/documents/my-doc/comments/c1/attachments/image-1)\nOriginal',
    );
    expect(wrapper.get('[data-comment-attachment-announcement]').text()).toBe('Comment saved');
  });

  it('binds attachment-list retry to the current published document comment', async () => {
    setup([comment('c1', 'Mine', 'me', 'user', 'Me')]);
    signInAs('me');

    const wrapper = mountPanel();
    const retry = wrapper.getComponent(CommentCard).props('onReloadAttachments') as () => Promise<void>;
    await retry();

    expect(commentAttachments.reload).toHaveBeenCalledWith('c1');
  });

  it('announces completed document attachment upload, download, and delete through the host card flow', async () => {
    setup([comment('c1', 'Mine', 'me', 'user', 'Me')]);
    commentAttachments.items.value = {
      c1: [
        {
          id: 'attachment-1',
          comment_id: 'c1',
          content_type: 'text/plain',
          created_at: '2026-01-01T00:00:00Z',
          file_name: 'notes.txt',
          size_bytes: 12,
        },
      ],
    };
    commentAttachments.upload.mockResolvedValue({ id: 'attachment-2' });
    commentAttachments.download.mockResolvedValue(new Blob(['download']));
    commentAttachments.delete.mockResolvedValue(true);
    signInAs('me');

    const wrapper = mountPanel();
    const picker = wrapper.get('[data-comment-attachment-picker]');
    Object.defineProperty(picker.element, 'files', {
      configurable: true,
      value: [new File(['upload'], 'upload.txt', { type: 'text/plain' })],
    });
    await picker.trigger('change');
    await flushPromises();
    expect(wrapper.get('[data-comment-attachment-announcement]').text()).toBe('Attachment uploaded');

    await wrapper.get('[aria-label="Download notes.txt"]').trigger('click');
    await flushPromises();
    expect(wrapper.get('[data-comment-attachment-announcement]').text()).toBe('Attachment downloaded');

    await wrapper.get('[aria-label="Delete notes.txt"]').trigger('click');
    await wrapper.get('[data-test="confirm"]').trigger('click');
    await flushPromises();
    expect(wrapper.get('[data-comment-attachment-announcement]').text()).toBe('Attachment deleted');
  });

  it('keeps document attachment failures actionable without a success announcement', async () => {
    setup([comment('c1', 'Mine', 'me', 'user', 'Me')]);
    commentAttachments.items.value = {
      c1: [
        {
          id: 'attachment-1',
          comment_id: 'c1',
          content_type: 'text/plain',
          created_at: '2026-01-01T00:00:00Z',
          file_name: 'notes.txt',
          size_bytes: 12,
        },
      ],
    };
    commentAttachments.error.value = { c1: 'Attachment operation failed. Retry attachment load.' };
    commentAttachments.upload.mockResolvedValue(null);
    commentAttachments.download.mockResolvedValue(null);
    commentAttachments.delete.mockResolvedValue(false);
    signInAs('me');

    const wrapper = mountPanel();
    const picker = wrapper.get('[data-comment-attachment-picker]');
    Object.defineProperty(picker.element, 'files', {
      configurable: true,
      value: [new File(['upload'], 'upload.txt', { type: 'text/plain' })],
    });
    await picker.trigger('change');
    await wrapper.get('[aria-label="Download notes.txt"]').trigger('click');
    await wrapper.get('[aria-label="Delete notes.txt"]').trigger('click');
    await wrapper.get('[data-test="confirm"]').trigger('click');
    await flushPromises();

    expect(wrapper.get('[data-comment-id="c1"] [role="alert"]').text()).toContain(
      'Attachment operation failed',
    );
    expect(wrapper.find('[data-comment-attachment-announcement]').exists()).toBe(false);
  });

  it("lets a workspace admin delete but not edit another member's comment", async () => {
    setup([comment('c1', 'Theirs', 'someone-else')]);
    signInAs('admin', 'admin');

    const wrapper = mountPanel();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');

    expect(menuItem(wrapper, 'Edit')).toBeUndefined();
    expect(menuItem(wrapper, 'Delete')).toBeDefined();
  });

  it('deletes a comment via the actions menu after confirmation', async () => {
    const store = setup([comment('c1', 'Mine', 'me', 'user', 'Me')]);
    const removeComment = vi.spyOn(store, 'removeComment').mockResolvedValue(true);
    signInAs('me');

    const wrapper = mountPanel();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');
    await menuItem(wrapper, 'Delete')?.trigger('click');
    await wrapper.get('[data-test="confirm"]').trigger('click');
    await flushPromises();

    expect(removeComment).toHaveBeenCalledWith('acme', 'my-doc', 'c1');
  });

  it('saves an inline edit via editComment and exits edit mode', async () => {
    const store = setup([comment('c1', 'Original', 'me', 'user', 'Me')]);
    const editComment = vi.spyOn(store, 'editComment').mockResolvedValue(true);
    signInAs('me');

    const wrapper = mountPanel();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');
    await menuItem(wrapper, 'Edit')?.trigger('click');

    const editor = wrapper.get('[data-comment-id="c1"] textarea');
    expect((editor.element as HTMLTextAreaElement).value).toBe('Original');

    await editor.setValue('Reworded');
    await wrapper.get('[data-comment-id="c1"] [data-test="comment-edit-save"]').trigger('click');
    await flushPromises();

    expect(editComment).toHaveBeenCalledWith('acme', 'my-doc', 'c1', 'Reworded');
    expect(wrapper.find('[data-comment-id="c1"] [data-test="comment-edit-save"]').exists()).toBe(false);
  });

  it('preserves verbatim Markdown when editing a comment', async () => {
    const store = setup([comment('c1', 'Original', 'me', 'user', 'Me')]);
    const editComment = vi.spyOn(store, 'editComment').mockResolvedValue(true);
    const body = '  leading\n```md\ncode\n```\ntrailing  ';

    signInAs('me');

    const wrapper = mountPanel();
    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');
    await menuItem(wrapper, 'Edit')?.trigger('click');
    await wrapper.get('[data-comment-id="c1"] textarea').setValue(body);
    await wrapper.get('[data-comment-id="c1"] [data-test="comment-edit-save"]').trigger('click');
    await flushPromises();

    expect(editComment).toHaveBeenCalledWith('acme', 'my-doc', 'c1', body);
  });

  it('loads the next shared-feed page when more remain', async () => {
    setup([comment('c1', 'First', 'u1')], true);

    const wrapper = mountPanel();

    await wrapper.get('[data-test="comment-load-more"]').trigger('click');

    expect(commentFeed.loadMore).toHaveBeenCalledWith({ kind: 'document', ws: 'acme', slug: 'my-doc' });
  });
});
