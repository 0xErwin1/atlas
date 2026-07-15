import { type DOMWrapper, flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick, ref } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import ActivityComments from '@/components/tareas/ActivityComments.vue';
import { useAuthStore } from '@/stores/auth';
import {
  type ActivityEntryDto,
  type CommentDto,
  type ReferenceDto,
  useTaskDetailStore,
} from '@/stores/taskDetail';
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

const RouteStub = { template: '<div />' };
const testRouter = createRouter({
  history: createMemoryHistory(),
  routes: [
    { path: '/t/task/:readableId', name: 'task-detail', component: RouteStub },
    { path: '/n/:slug?', name: 'notes', component: RouteStub },
  ],
});

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
  task_id: 't1',
  body,
  author: { id: authorId, type: authorType, display_name: name },
  created_at: createdAt,
  updated_at: updatedAt,
});

const activity = (
  id: string,
  kind: string,
  createdAt: string,
  name: string | null = 'Robin',
  type = 'user',
): ActivityEntryDto => ({
  id,
  kind,
  actor: { id: `actor-${id}`, type, display_name: name },
  created_at: createdAt,
  payload: null,
  task_id: 't1',
  task_readable_id: 'ATL-1',
});

function mountFeed() {
  if (commentFeed.entries.value.length === 0) {
    commentFeed.entries.value = useTaskDetailStore().comments.map((loadedComment) => ({
      type: 'comment',
      comment: loadedComment,
      links: [],
    }));
  }
  commentFeed.hasMore.value = useTaskDetailStore().commentsHasMore;

  return mount(ActivityComments, {
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
  commentFeed.entries.value = [];
  commentFeed.hasMore.value = false;
  commentFeed.status.value = 'ready';
  commentFeed.error.value = null;
  commentAttachments.items.value = {};
  commentAttachments.error.value = {};
  vi.clearAllMocks();
});

describe('ActivityComments feed (ATL-19)', () => {
  it('renders shared comments, retained events, and existing activity in chronological order', async () => {
    useTaskDetailStore()._setForTest({
      activity: [activity('a1', 'created', '2026-01-01T09:00:00Z')],
    });
    commentFeed.entries.value = [
      {
        type: 'comment',
        comment: comment('c1', 'Linked note', 'u1', 'user', 'Jordan', '2026-01-01T10:00:00Z'),
        links: [{ target: { status: 'available', id: 'ATL-9', type: 'task', label: 'ATL-9' } }],
      },
      {
        type: 'event',
        id: 'e1',
        kind: 'link_added',
        created_at: '2026-01-01T11:00:00Z',
        target: { status: 'unavailable', label: 'Recurso no disponible' },
      },
    ];

    const wrapper = mountFeed();
    await flushPromises();

    expect(commentFeed.load).toHaveBeenCalledWith({ kind: 'task', ws: 'acme', readableId: 'ATL-1' });
    expect(
      wrapper
        .findAll('[data-activity-id], [data-comment-id], [data-comment-event]')
        .map(
          (node) =>
            node.attributes('data-activity-id') ??
            node.attributes('data-comment-id') ??
            node.attributes('data-comment-event'),
        ),
    ).toEqual(['a1', 'c1', 'e1']);
    expect(wrapper.get('[data-comment-link="ATL-9"]').text()).toBe('ATL-9');
    expect(wrapper.get('[data-comment-event="e1"]').text()).toContain('Recurso no disponible');
  });

  it('navigates available shared task links without exposing unavailable target metadata', async () => {
    commentFeed.entries.value = [
      {
        type: 'comment',
        comment: comment('c1', 'Linked task', 'u1'),
        links: [
          { target: { status: 'available', id: 'ATL-9', type: 'task', label: 'ATL-9' } },
          { target: { status: 'unavailable', label: 'Recurso no disponible', id: 'hidden-title' } },
        ],
      },
    ];

    const wrapper = mount(ActivityComments, {
      props: { ws: 'acme', readableId: 'ATL-1' },
      global: { plugins: [testRouter], stubs: { MarkdownEditor: MarkdownEditorStub, teleport: true } },
    });
    await wrapper.get('[data-comment-link="ATL-9"]').trigger('click');
    await flushPromises();

    expect(testRouter.currentRoute.value.fullPath).toBe('/t/task/ATL-9');
    expect(wrapper.text()).toContain('Recurso no disponible');
    expect(wrapper.text()).not.toContain('hidden-title');
  });
  it('interleaves activity entries and comments in chronological order', () => {
    useTaskDetailStore()._setForTest({
      activity: [
        activity('a1', 'created', '2026-01-01T09:00:00Z'),
        activity('a2', 'moved', '2026-01-01T11:00:00Z'),
      ],
      comments: [comment('c1', 'Middle note', 'u1', 'user', 'Jordan', '2026-01-01T10:00:00Z')],
    });

    const wrapper = mountFeed();

    const order = wrapper
      .findAll('[data-activity-id], [data-comment-id]')
      .map((n) => n.attributes('data-activity-id') ?? n.attributes('data-comment-id'));
    expect(order).toEqual(['a1', 'c1', 'a2']);
  });

  it('renders an activity entry as a readable line', () => {
    useTaskDetailStore()._setForTest({
      activity: [activity('a1', 'created', '2026-01-01T09:00:00Z', 'Robin')],
      comments: [],
    });

    const wrapper = mountFeed();

    const row = wrapper.get('[data-activity-id="a1"]');
    expect(row.text()).toContain('Robin');
    expect(row.text()).toContain('created this task');
  });

  it('renders a card per comment with its author and body', () => {
    useTaskDetailStore()._setForTest({
      comments: [
        comment('c1', 'First note', 'u1', 'user', 'Jordan'),
        comment('c2', 'Second note', 'k1', 'api_key', 'Claude'),
      ],
    });

    const wrapper = mountFeed();

    const cards = wrapper.findAll('[data-comment-id]');
    expect(cards).toHaveLength(2);
    expect(cards[0]?.text()).toContain('Jordan');
    expect(cards[0]?.text()).toContain('First note');
    expect(cards[1]?.text()).toContain('Claude');
    expect(cards[1]?.text()).toContain('Second note');
  });

  it('shows an "(edited)" marker when the comment was updated after creation', () => {
    useTaskDetailStore()._setForTest({
      comments: [
        comment('c1', 'Untouched', 'u1'),
        comment('c2', 'Reworded', 'u1', 'user', 'Jordan', '2026-01-01T00:00:00Z', '2026-02-02T00:00:00Z'),
      ],
    });

    const wrapper = mountFeed();

    expect(wrapper.get('[data-comment-id="c1"]').text()).not.toContain('(edited)');
    expect(wrapper.get('[data-comment-id="c2"]').text()).toContain('(edited)');
  });

  it('shows a compact empty state when there is no activity or comments', () => {
    useTaskDetailStore()._setForTest({ comments: [], activity: [] });

    const wrapper = mountFeed();

    expect(wrapper.find('[data-comment-id]').exists()).toBe(false);
    expect(wrapper.find('[data-state="empty"]').exists()).toBe(true);
    expect(wrapper.text()).toContain('No activity yet');
  });

  it('submits the typed body via addComment and clears the composer', async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [] });
    const addComment = vi.spyOn(detail, 'addComment').mockResolvedValue(true);

    const wrapper = mountFeed();

    const input = wrapper.get('[data-comment-composer] textarea');
    await input.setValue('New comment');
    await wrapper.get('[data-test="comment-submit"]').trigger('click');
    await flushPromises();

    expect(addComment).toHaveBeenCalledWith('acme', 'ATL-1', 'New comment');
    expect((input.element as HTMLTextAreaElement).value).toBe('');
  });

  it('focuses the editor when the composer box is clicked', async () => {
    useTaskDetailStore()._setForTest({ comments: [] });

    const wrapper = mountFeed();

    await wrapper.get('[data-comment-composer]').trigger('click');

    expect(editorFocus).toHaveBeenCalled();
  });

  it('disables submit for an empty or whitespace-only body', async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [] });
    const addComment = vi.spyOn(detail, 'addComment').mockResolvedValue(true);

    const wrapper = mountFeed();

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

    const wrapper = mountFeed();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');
    const del = menuItem(wrapper, 'Delete');
    expect(del).toBeDefined();
    await del?.trigger('click');
    await wrapper.get('[data-test="confirm"]').trigger('click');
    await flushPromises();

    expect(removeComment).toHaveBeenCalledWith('acme', 'ATL-1', 'c1');
  });

  it('offers Edit and Delete to the comment author', async () => {
    useTaskDetailStore()._setForTest({ comments: [comment('c1', 'Mine', 'me', 'user', 'Me')] });

    signInAs('me');

    const wrapper = mountFeed();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');

    expect(menuItem(wrapper, 'Edit')).toBeDefined();
    expect(menuItem(wrapper, 'Delete')).toBeDefined();
  });

  it("lets a workspace admin delete but not edit another member's comment", async () => {
    useTaskDetailStore()._setForTest({ comments: [comment('c1', 'Theirs', 'someone-else')] });

    signInAs('admin', 'admin');

    const wrapper = mountFeed();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');

    expect(menuItem(wrapper, 'Edit')).toBeUndefined();
    expect(menuItem(wrapper, 'Delete')).toBeDefined();
  });

  it('hides the actions menu for a comment the member neither authored nor can moderate', () => {
    useTaskDetailStore()._setForTest({ comments: [comment('c1', 'Theirs', 'someone-else')] });

    signInAs('me');

    const wrapper = mountFeed();

    expect(wrapper.find('[data-comment-id="c1"] [aria-label="Comment actions"]').exists()).toBe(false);
  });

  it('saves an inline edit via editComment and exits edit mode', async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [comment('c1', 'Original', 'me', 'user', 'Me')] });
    const editComment = vi.spyOn(detail, 'editComment').mockResolvedValue(true);

    signInAs('me');

    const wrapper = mountFeed();

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

  it('preserves verbatim Markdown when creating a comment', async () => {
    const detail = useTaskDetailStore();
    const addComment = vi.spyOn(detail, 'addComment').mockResolvedValue(true);
    const body = '  leading\n```md\ncode\n```\ntrailing  ';

    signInAs('me');

    const wrapper = mountFeed();
    await wrapper.get('[data-comment-composer] textarea').setValue(body);
    await wrapper.get('[data-test="comment-submit"]').trigger('click');
    await flushPromises();

    expect(addComment).toHaveBeenCalledWith('acme', 'ATL-1', body);
  });

  it('discards an inline edit on cancel without calling editComment', async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [comment('c1', 'Original', 'me', 'user', 'Me')] });
    const editComment = vi.spyOn(detail, 'editComment').mockResolvedValue(true);

    signInAs('me');

    const wrapper = mountFeed();

    await wrapper.get('[data-comment-id="c1"] [aria-label="Comment actions"]').trigger('click');
    await menuItem(wrapper, 'Edit')?.trigger('click');

    await wrapper.get('[data-comment-id="c1"] textarea').setValue('Reworded');
    await wrapper.get('[data-comment-id="c1"] [data-test="comment-edit-cancel"]').trigger('click');

    expect(editComment).not.toHaveBeenCalled();
    expect(wrapper.find('[data-comment-id="c1"] [data-test="comment-edit-save"]').exists()).toBe(false);
  });

  it('in pinned mode docks the composer and lands at the end of the feed on open', async () => {
    useTaskDetailStore()._setForTest({
      activity: [activity('a1', 'created', '2026-01-01T09:00:00Z')],
      comments: [comment('c1', 'Note', 'u1', 'user', 'Jordan', '2026-01-01T10:00:00Z')],
    });

    const wrapper = mount(ActivityComments, {
      props: { ws: 'acme', readableId: 'ATL-1', pinned: true },
      global: { stubs: { MarkdownEditor: MarkdownEditorStub, teleport: true } },
    });

    expect(wrapper.get('.atl-ac').classes()).toContain('pinned');
    expect(wrapper.find('.atl-ac-composer [data-comment-composer]').exists()).toBe(true);

    // jsdom does no layout, so stand in a scrollable feed before the queued
    // scroll-to-end runs and assert it jumps to the bottom.
    const scroll = wrapper.get('.atl-ac-scroll').element as HTMLElement;
    Object.defineProperty(scroll, 'scrollHeight', { value: 640, configurable: true });

    await nextTick();

    expect(scroll.scrollTop).toBe(640);
  });

  it('links a reference_added entry to the referenced task (ATL-65)', () => {
    const reference: ReferenceDto = {
      id: 'r1',
      origins: ['manual'],
      wikilink_reference_id: null,
      manual_reference_id: 'r1',
      manual_kind: 'relates',
      manual_created_at: '2026-01-01T00:00:00Z',
      manual_created_by: { id: 'u1', type: 'user', display_name: 'Robin' },
      target_task_id: 't9',
      target_document_id: null,
      target_title: null,
      target_readable_id: 'ATL-9',
      target_resolved: true,
    };

    const entry: ActivityEntryDto = {
      id: 'a1',
      kind: 'reference_added',
      actor: { id: 'actor-a1', type: 'user', display_name: 'Robin' },
      created_at: '2026-01-01T09:00:00Z',
      payload: { reference_added: { reference_id: 'r1', kind: 'relates' } },
      task_id: 't1',
      task_readable_id: 'ATL-1',
    };

    useTaskDetailStore()._setForTest({ activity: [entry], references: [reference] });

    const wrapper = mount(ActivityComments, {
      props: { ws: 'acme', readableId: 'ATL-1' },
      global: {
        plugins: [testRouter],
        stubs: { MarkdownEditor: MarkdownEditorStub, teleport: true },
      },
    });

    const link = wrapper.get('[data-activity-id="a1"] a.atl-ac-reflink');
    expect(link.text()).toBe('ATL-9');
    expect(link.attributes('href')).toBe('/t/task/ATL-9');
  });

  it('loads the next shared-feed page when more remain', async () => {
    const detail = useTaskDetailStore();
    detail._setForTest({ comments: [comment('c1', 'First', 'u1')], commentsHasMore: true });

    const wrapper = mountFeed();

    await wrapper.get('[data-test="comment-load-more"]').trigger('click');

    expect(commentFeed.loadMore).toHaveBeenCalledWith({ kind: 'task', ws: 'acme', readableId: 'ATL-1' });
  });
});
