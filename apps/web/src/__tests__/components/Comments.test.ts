import { type DOMWrapper, flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Comments from '@/components/tareas/Comments.vue';
import { useAuthStore } from '@/stores/auth';
import { type CommentDto, useTaskDetailStore } from '@/stores/taskDetail';
import { useWorkspaceStore } from '@/stores/workspace';

const editorFocus = vi.fn();

/**
 * Stubs the CodeMirror-backed editor so the panel can be mounted in jsdom without
 * a real editor instance: it renders the markdown body as plain text (so the
 * read-only comment body is assertable), exposes a textarea that re-emits the
 * host's `change` event (so drafts can be typed), and exposes a `focus` spy that
 * mirrors the real editor's `defineExpose` (so the composer click-to-focus wiring
 * is assertable).
 */
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
  updatedAt = '2026-01-01T00:00:00Z',
): CommentDto => ({
  id,
  task_id: 't1',
  body,
  author: { id: authorId, type: authorType, display_name: name },
  created_at: '2026-01-01T00:00:00Z',
  updated_at: updatedAt,
});

function mountComments() {
  return mount(Comments, {
    props: { ws: 'acme', readableId: 'ATL-1' },
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

  it('shows an "(edited)" marker when the comment was updated after creation', () => {
    const detail = useTaskDetailStore();
    detail._setForTest({
      comments: [
        comment('c1', 'Untouched', 'u1'),
        comment('c2', 'Reworded', 'u1', 'user', 'Jordan', '2026-02-02T00:00:00Z'),
      ],
    });

    const wrapper = mountComments();

    expect(wrapper.get('[data-comment-id="c1"]').text()).not.toContain('(edited)');
    expect(wrapper.get('[data-comment-id="c2"]').text()).toContain('(edited)');
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

  it('focuses the editor when the composer box is clicked', async () => {
    useTaskDetailStore()._setForTest({ comments: [] });

    const wrapper = mountComments();

    await wrapper.get('[data-comment-composer]').trigger('click');

    expect(editorFocus).toHaveBeenCalled();
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

  it('deletes a comment via the actions menu after confirmation', async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [comment('c1', 'Mine', 'me', 'user', 'Me')] });
    const removeComment = vi.spyOn(detail, 'removeComment').mockResolvedValue(true);

    signInAs('me');

    const wrapper = mountComments();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');
    const del = menuItem(wrapper, 'Delete');
    expect(del).toBeDefined();
    await del?.trigger('click');
    await wrapper.get('[data-test="confirm"]').trigger('click');
    await flushPromises();

    expect(removeComment).toHaveBeenCalledWith('acme', 'ATL-1', 'c1');
  });

  it('offers Edit and Delete to the comment author', async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [comment('c1', 'Mine', 'me', 'user', 'Me')] });

    signInAs('me');

    const wrapper = mountComments();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');

    expect(menuItem(wrapper, 'Edit')).toBeDefined();
    expect(menuItem(wrapper, 'Delete')).toBeDefined();
  });

  it("lets a workspace admin delete but not edit another member's comment", async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [comment('c1', 'Theirs', 'someone-else')] });

    signInAs('admin', 'admin');

    const wrapper = mountComments();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');

    expect(menuItem(wrapper, 'Edit')).toBeUndefined();
    expect(menuItem(wrapper, 'Delete')).toBeDefined();
  });

  it('hides the actions menu for a comment the member neither authored nor can moderate', () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [comment('c1', 'Theirs', 'someone-else')] });

    signInAs('me');

    const wrapper = mountComments();

    expect(wrapper.find('[data-comment-id="c1"] [aria-label="Comment actions"]').exists()).toBe(false);
  });

  it('saves an inline edit via editComment and exits edit mode', async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [comment('c1', 'Original', 'me', 'user', 'Me')] });
    const editComment = vi.spyOn(detail, 'editComment').mockResolvedValue(true);

    signInAs('me');

    const wrapper = mountComments();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');
    await menuItem(wrapper, 'Edit')?.trigger('click');

    const editor = wrapper.get('[data-comment-id="c1"] textarea');
    expect((editor.element as HTMLTextAreaElement).value).toBe('Original');

    await editor.setValue('Reworded');
    await wrapper.get('[data-comment-id="c1"] [data-test="comment-edit-save"]').trigger('click');
    await flushPromises();

    expect(editComment).toHaveBeenCalledWith('acme', 'ATL-1', 'c1', 'Reworded');
    expect(wrapper.find('[data-comment-id="c1"] [data-test="comment-edit-save"]').exists()).toBe(false);
  });

  it('discards an inline edit on cancel without calling editComment', async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [comment('c1', 'Original', 'me', 'user', 'Me')] });
    const editComment = vi.spyOn(detail, 'editComment').mockResolvedValue(true);

    signInAs('me');

    const wrapper = mountComments();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');
    await menuItem(wrapper, 'Edit')?.trigger('click');

    await wrapper.get('[data-comment-id="c1"] textarea').setValue('Reworded');
    await wrapper.get('[data-comment-id="c1"] [data-test="comment-edit-cancel"]').trigger('click');

    expect(editComment).not.toHaveBeenCalled();
    expect(wrapper.find('[data-comment-id="c1"] [data-test="comment-edit-save"]').exists()).toBe(false);
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
