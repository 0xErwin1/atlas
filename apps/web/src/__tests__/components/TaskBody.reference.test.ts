import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET: vi.fn().mockResolvedValue({ data: undefined }),
    POST: vi.fn(),
    PATCH: vi.fn(),
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

const attachment = {
  id: 'att-1',
  file_name: 'diagram.png',
  content_type: 'image/png',
  size_bytes: 10,
  created_by: task.created_by,
  created_at: '2026-01-01T00:00:00Z',
};

describe('TaskBody attachment references', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    URL.createObjectURL = vi.fn(() => 'blob:1');
    URL.revokeObjectURL = vi.fn();
  });

  it('writes the reference into the description and persists it', async () => {
    const detail = useTaskDetailStore();
    const tags = useTagsStore();
    const tasks = useTasksStore();
    vi.spyOn(tags, 'load').mockResolvedValue();
    const updateDescription = vi.spyOn(tasks, 'updateDescription').mockResolvedValue(true);

    detail._setForTest({ attachments: [attachment] });
    detail.collectionStatus = { ...detail.collectionStatus, attachments: 'ready' };
    detail.collectionLoaded = { ...detail.collectionLoaded, attachments: true };

    const wrapper = mount(TaskBody, {
      props: { task, ws: 'acme' },
      global: { stubs: { RouterLink: true } },
    });
    await flushPromises();

    vi.useFakeTimers();
    await wrapper.get('[aria-label="Reference diagram.png in the description"]').trigger('click');
    vi.advanceTimersByTime(1000);
    vi.useRealTimers();

    expect(updateDescription).toHaveBeenCalledWith(
      'acme',
      'ATL-1',
      '![diagram](/api/workspaces/acme/tasks/ATL-1/attachments/att-1/content)\n',
    );
  });
});
