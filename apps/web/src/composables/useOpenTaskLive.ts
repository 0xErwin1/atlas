import type { Ref } from 'vue';
import { EVENT_TYPE } from '@/lib/eventTypes';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useTasksStore } from '@/stores/tasks';

export type OpenTaskOutcome = 'deleted' | 'refreshed' | 'ignored';

/**
 * Applies a task-level live event to the currently open task, shared by the
 * board's inline detail pane and the standalone task route so the reaction is
 * defined once.
 *
 * The event carries the task's UUID; the open task holds both its UUID and its
 * readable id, so a match reloads the detail and its related collections by
 * readable id. A `task.deleted` for the open task clears it and returns
 * `deleted` so the caller can dismiss its surface. Any event whose task is not
 * the open one is ignored.
 */
export function useOpenTaskLive(ws: Ref<string>): {
  apply: (type: string, taskId: string) => OpenTaskOutcome;
} {
  const tasks = useTasksStore();
  const detail = useTaskDetailStore();

  function apply(type: string, taskId: string): OpenTaskOutcome {
    const open = tasks.openTask;
    if (open === null || open.id !== taskId) return 'ignored';

    if (type === EVENT_TYPE.TASK_DELETED) {
      tasks.clear();
      return 'deleted';
    }

    void tasks.loadTask(ws.value, open.readable_id);
    void detail.loadAll(ws.value, open.readable_id);
    return 'refreshed';
  }

  return { apply };
}
