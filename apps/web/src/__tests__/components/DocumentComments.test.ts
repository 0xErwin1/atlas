import { type DOMWrapper, flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ref } from 'vue';
import DocumentComments from '@/components/notas/DocumentComments.vue';
import { useAuthStore } from '@/stores/auth';
import { type CommentDto, useDocumentsStore } from '@/stores/documents';
import { useWorkspaceStore } from '@/stores/workspace';

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
    global: { stubs: { MarkdownEditor: MarkdownEditorStub, teleport: true } },
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
