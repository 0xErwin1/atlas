import { flushPromises, shallowMount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import TaskBody from '@/components/tareas/TaskBody.vue';
import { useTagsStore } from '@/stores/tags';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useUiStore } from '@/stores/ui';

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

describe('TaskBody attachment picker', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('selects and uploads multiple files sequentially while preserving batch feedback', async () => {
    const detail = useTaskDetailStore();
    const tags = useTagsStore();
    const ui = useUiStore();
    const uploadOrder: string[] = [];
    let activeUploads = 0;
    let maxActiveUploads = 0;

    const uploadAttachment = vi
      .spyOn(detail, 'uploadAttachment')
      .mockImplementation(async (_ws, _taskId, file) => {
        uploadOrder.push(file.name);
        activeUploads += 1;
        maxActiveUploads = Math.max(maxActiveUploads, activeUploads);
        await Promise.resolve();
        activeUploads -= 1;

        if (file.name === 'broken.txt') {
          detail.error = 'Upload failed';
          return false;
        }

        return true;
      });
    vi.spyOn(tags, 'load').mockResolvedValue();
    const showBanner = vi.spyOn(ui, 'showBanner');
    const wrapper = shallowMount(TaskBody, { props: { task, ws: 'ws' } });
    const input = wrapper.get('input[type="file"]');
    const files = [
      new File(['first'], 'first.txt'),
      new File(['broken'], 'broken.txt'),
      new File(['last'], 'last.txt'),
    ];

    expect(input.attributes('multiple')).toBeDefined();

    Object.defineProperty(input.element, 'files', { configurable: true, value: files });
    await input.trigger('change');
    await flushPromises();

    expect(uploadOrder).toEqual(['first.txt', 'broken.txt', 'last.txt']);
    expect(maxActiveUploads).toBe(1);
    expect(input.element).toHaveProperty('value', '');
    expect(showBanner).toHaveBeenCalledWith('Upload failed', 'error');
    expect(showBanner).toHaveBeenCalledWith('2 attachments uploaded', 'success');
    expect(wrapper.get('.atl-tv-attach').attributes('disabled')).toBeUndefined();
    expect(wrapper.get('.atl-tv-attach').text()).toContain('Attach file');

    Object.defineProperty(input.element, 'files', { configurable: true, value: [files[0]] });
    await input.trigger('change');
    await flushPromises();

    expect(uploadAttachment).toHaveBeenCalledTimes(4);
    expect(uploadAttachment).toHaveBeenLastCalledWith('ws', 'ATL-1', files[0]);
  });
});
