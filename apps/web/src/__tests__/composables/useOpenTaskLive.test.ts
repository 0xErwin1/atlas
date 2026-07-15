import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ref } from 'vue';
import { useOpenTaskLive } from '@/composables/useOpenTaskLive';
import { EVENT_TYPE } from '@/lib/eventTypes';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useTasksStore } from '@/stores/tasks';

describe('useOpenTaskLive', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('reloads an open task detail with its authoritative UUID', () => {
    const tasks = useTasksStore();
    const detail = useTaskDetailStore();
    const loadAll = vi.spyOn(detail, 'loadAll').mockResolvedValue();
    tasks.openTask = {
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

    const outcome = useOpenTaskLive(ref('ws')).apply(EVENT_TYPE.TASK_UPDATED, 'task-1');

    expect(outcome).toBe('refreshed');
    expect(loadAll).toHaveBeenCalledWith('ws', 'ATL-1', 'workspace-1', 'task-1');
  });

  it('does not reload detail for an unrelated task event', () => {
    const tasks = useTasksStore();
    const detail = useTaskDetailStore();
    const loadAll = vi.spyOn(detail, 'loadAll').mockResolvedValue();
    tasks.openTask = {
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

    const outcome = useOpenTaskLive(ref('ws')).apply(EVENT_TYPE.TASK_UPDATED, 'task-2');

    expect(outcome).toBe('ignored');
    expect(loadAll).not.toHaveBeenCalled();
  });
});
