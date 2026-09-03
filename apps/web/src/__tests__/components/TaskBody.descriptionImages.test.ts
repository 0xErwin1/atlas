import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET: vi.fn().mockResolvedValue({ data: undefined }),
    POST: vi.fn(),
    // The description auto-save PATCHes through the real store once its debounce
    // fires; resolve to the shape the store destructures so it settles cleanly.
    PATCH: vi.fn().mockResolvedValue({ data: undefined }),
    DELETE: vi.fn(),
  },
}));

import TaskBody from '@/components/tareas/TaskBody.vue';
import { useTagsStore } from '@/stores/tags';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useTasksStore } from '@/stores/tasks';

const task = {
  id: 'task-1',
  readable_id: 'ATL-1',
  board_id: 'board-1',
  board_name: 'Board',
  column_id: 'column-1',
  column_name: 'Todo',
  title: 'Task',
  description: '',
  project_id: 'project-1',
  workspace_id: 'workspace-1',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-02T00:00:00Z',
  created_by: { id: 'user-1', type: 'user', display_name: 'Jordan' },
};

const uploaded = {
  id: 'att-1',
  file_name: 'shot.png',
  content_type: 'image/png',
  size_bytes: 4,
  created_by: task.created_by,
  created_at: '2026-01-01T00:00:00Z',
};

function pasteImage(target: Element, file: File): void {
  const event = new Event('paste', { bubbles: true, cancelable: true });
  Object.defineProperty(event, 'clipboardData', {
    value: { items: [{ kind: 'file', getAsFile: () => file }], getData: () => '' },
  });
  target.dispatchEvent(event);
}

describe('TaskBody description images', () => {
  let mounted: VueWrapper | null = null;

  beforeEach(() => {
    setActivePinia(createPinia());
    URL.createObjectURL = vi.fn(() => 'blob:1');
    URL.revokeObjectURL = vi.fn();
  });

  // Unmounting flushes the description's pending debounced save inside the
  // test, so no timer outlives the test and fires after mocks are torn down.
  afterEach(async () => {
    mounted?.unmount();
    mounted = null;
    await flushPromises();
  });

  async function body() {
    const tags = useTagsStore();
    vi.spyOn(tags, 'load').mockResolvedValue();

    const wrapper = mount(TaskBody, {
      props: { task, ws: 'acme' },
      global: { stubs: { RouterLink: true } },
    });
    mounted = wrapper;
    await flushPromises();
    return wrapper;
  }

  it('attaches an image pasted into the description and embeds it inline', async () => {
    const detail = useTaskDetailStore();
    const tasks = useTasksStore();
    const upload = vi.spyOn(detail, 'uploadAttachment').mockResolvedValue(uploaded);
    const updateDescription = vi.spyOn(tasks, 'updateDescription').mockResolvedValue(true);
    const file = new File(['png'], 'shot.png', { type: 'image/png' });

    // The debounce is armed while the upload promise settles, so the clock must
    // already be faked before the paste, yet still advance for `flushPromises`.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const wrapper = await body();
    pasteImage(wrapper.get('.cm-content').element, file);
    await flushPromises();

    expect(upload).toHaveBeenCalledWith('acme', 'ATL-1', file);

    vi.advanceTimersByTime(1000);
    vi.useRealTimers();

    expect(updateDescription).toHaveBeenCalledWith(
      'acme',
      'ATL-1',
      '![shot](/api/v2/acta/workspaces/acme/tasks/ATL-1/attachments/att-1/content)\n',
    );
  });

  it('uploads a description paste once, not again through the surrounding dropzone', async () => {
    const detail = useTaskDetailStore();
    const upload = vi.spyOn(detail, 'uploadAttachment').mockResolvedValue(uploaded);
    const file = new File(['png'], 'shot.png', { type: 'image/png' });

    const wrapper = await body();
    pasteImage(wrapper.get('.cm-content').element, file);
    await flushPromises();

    expect(upload).toHaveBeenCalledTimes(1);
  });

  it('still attaches an image pasted outside the description', async () => {
    const detail = useTaskDetailStore();
    const upload = vi.spyOn(detail, 'uploadAttachment').mockResolvedValue(uploaded);
    const file = new File(['png'], 'shot.png', { type: 'image/png' });

    await body();
    pasteImage(document.body, file);
    await flushPromises();

    expect(upload).toHaveBeenCalledWith('acme', 'ATL-1', file);
  });
});
