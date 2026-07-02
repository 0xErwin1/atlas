import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Comments from '@/components/tareas/Comments.vue';
import { useAuthStore } from '@/stores/auth';
import { type CommentDto, useTaskDetailStore } from '@/stores/taskDetail';
import { useWorkspaceStore } from '@/stores/workspace';

/**
 * Stubs the CodeMirror-backed editor so the panel can be mounted in jsdom without
 * a real editor instance: it renders the markdown body as plain text (so the
 * read-only comment body is assertable) and exposes a textarea that re-emits the
 * host's `change` event (so the composer draft can be typed).
 */
const MarkdownEditorStub = {
  name: 'MarkdownEditor',
  props: ['body', 'editable', 'reading', 'embeddedControls', 'placeholder', 'minHeight', 'widthToggle'],
  emits: ['change'],
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
): CommentDto => ({
  id,
  task_id: 't1',
  body,
  author: { id: authorId, type: authorType, display_name: name },
  created_at: '2026-01-01T00:00:00Z',
});

function mountComments() {
  return mount(Comments, {
    props: { ws: 'acme', readableId: 'ATL-1' },
    global: { stubs: { MarkdownEditor: MarkdownEditorStub, teleport: true } },
  });
}

beforeEach(() => {
  setActivePinia(createPinia());
});

describe('Comments panel (ATL-19)', () => {
  it('renders a card per comment with its author and body, oldest-first', () => {
    const detail = useTaskDetailStore();
    detail._setForTest({
      comments: [
        comment('c1', 'First note', 'u1', 'user', 'Jordan'),
        comment('c2', 'Second note', 'k1', 'api_key', 'Claude'),
      ],
    });

    const wrapper = mountComments();

    const cards = wrapper.findAll('[data-comment-id]');
    expect(cards).toHaveLength(2);
    expect(cards[0]?.text()).toContain('Jordan');
    expect(cards[0]?.text()).toContain('First note');
    expect(cards[1]?.text()).toContain('Claude');
    expect(cards[1]?.text()).toContain('Second note');
  });

  it('shows a compact empty state when there are no comments', () => {
    useTaskDetailStore()._setForTest({ comments: [] });

    const wrapper = mountComments();

    expect(wrapper.find('[data-comment-id]').exists()).toBe(false);
    expect(wrapper.find('[data-state="empty"]').exists()).toBe(true);
    expect(wrapper.text()).toContain('No comments yet');
  });

  it('submits the typed body via addComment and clears the composer', async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [] });
    const addComment = vi.spyOn(detail, 'addComment').mockResolvedValue(true);

    const wrapper = mountComments();

    const input = wrapper.get('[data-comment-composer] textarea');
    await input.setValue('New comment');
    await wrapper.get('[data-test="comment-submit"]').trigger('click');
    await flushPromises();

    expect(addComment).toHaveBeenCalledWith('acme', 'ATL-1', 'New comment');
    expect((input.element as HTMLTextAreaElement).value).toBe('');
  });

  it('disables submit for an empty or whitespace-only body', async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [] });
    const addComment = vi.spyOn(detail, 'addComment').mockResolvedValue(true);

    const wrapper = mountComments();

    const submit = wrapper.get('[data-test="comment-submit"]');
    expect((submit.element as HTMLButtonElement).disabled).toBe(true);

    await wrapper.get('[data-comment-composer] textarea').setValue('   ');
    expect((submit.element as HTMLButtonElement).disabled).toBe(true);

    await submit.trigger('click');
    expect(addComment).not.toHaveBeenCalled();
  });

  it('deletes a comment via removeComment after confirmation', async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [comment('c1', 'Mine', 'me', 'user', 'Me')] });
    const removeComment = vi.spyOn(detail, 'removeComment').mockResolvedValue(true);

    const auth = useAuthStore();
    auth.user = {
      id: 'me',
      username: 'me',
      display_name: 'Me',
      email: null,
      principal_type: 'user',
      is_root: false,
      is_system_admin: false,
    };

    const wrapper = mountComments();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Delete comment"]').trigger('click');
    await wrapper.get('[data-test="confirm"]').trigger('click');
    await flushPromises();

    expect(removeComment).toHaveBeenCalledWith('acme', 'ATL-1', 'c1');
  });

  it('hides the delete control for a comment the member neither authored nor can moderate', () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [comment('c1', 'Theirs', 'someone-else')] });

    const auth = useAuthStore();
    auth.user = {
      id: 'me',
      username: 'me',
      display_name: 'Me',
      email: null,
      principal_type: 'user',
      is_root: false,
      is_system_admin: false,
    };
    useWorkspaceStore().members = [
      {
        id: 'me',
        display: 'Me',
        principal_type: 'user',
        role: 'member',
        account_status: 'active',
        key_type: null,
      },
    ];

    const wrapper = mountComments();

    expect(wrapper.find('[data-comment-id="c1"] [aria-label="Delete comment"]').exists()).toBe(false);
  });

  it("lets a workspace admin delete another member's comment", () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [comment('c1', 'Theirs', 'someone-else')] });

    const auth = useAuthStore();
    auth.user = {
      id: 'admin',
      username: 'admin',
      display_name: 'Admin',
      email: null,
      principal_type: 'user',
      is_root: false,
      is_system_admin: false,
    };
    useWorkspaceStore().members = [
      {
        id: 'admin',
        display: 'Admin',
        principal_type: 'user',
        role: 'admin',
        account_status: 'active',
        key_type: null,
      },
    ];

    const wrapper = mountComments();

    expect(wrapper.find('[data-comment-id="c1"] [aria-label="Delete comment"]').exists()).toBe(true);
  });

  it('loads the next page via loadMoreComments when more remain', async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [comment('c1', 'First', 'u1')], commentsHasMore: true });
    const loadMore = vi.spyOn(detail, 'loadMoreComments').mockResolvedValue();

    const wrapper = mountComments();

    await wrapper.get('[data-test="comment-load-more"]').trigger('click');

    expect(loadMore).toHaveBeenCalledWith('acme', 'ATL-1');
  });
});
