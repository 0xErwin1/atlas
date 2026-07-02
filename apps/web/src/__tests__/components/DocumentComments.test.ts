import { type DOMWrapper, flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import DocumentComments from '@/components/notas/DocumentComments.vue';
import { useAuthStore } from '@/stores/auth';
import { type CommentDto, useDocumentsStore } from '@/stores/documents';
import { useWorkspaceStore } from '@/stores/workspace';

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
});

describe('DocumentComments (ATL-37)', () => {
  it('loads the thread for the open document on mount', () => {
    const store = setup([]);

    mountPanel();

    expect(store.loadComments).toHaveBeenCalledWith('acme', 'my-doc');
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

  it('loads the next page via loadMoreComments when more remain', async () => {
    const store = setup([comment('c1', 'First', 'u1')], true);
    const loadMore = vi.spyOn(store, 'loadMoreComments').mockResolvedValue();

    const wrapper = mountPanel();

    await wrapper.get('[data-test="comment-load-more"]').trigger('click');

    expect(loadMore).toHaveBeenCalledWith('acme', 'my-doc');
  });
});
