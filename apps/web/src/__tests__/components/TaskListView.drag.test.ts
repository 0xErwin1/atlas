import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('vue-draggable-plus', () => ({
  VueDraggable: {
    name: 'VueDraggable',
    props: [
      'modelValue',
      'scroll',
      'scrollSensitivity',
      'scrollSpeed',
      'bubbleScroll',
      'forceAutoScrollFallback',
      'onMove',
    ],
    template: '<div class="vdp-stub"><slot /></div>',
  },
}));

import TaskListView from '@/components/tareas/TaskListView.vue';
import { type ColumnDto, type TaskSummaryDto, useBoardsStore } from '@/stores/boards';

const column = (id: string, name: string): ColumnDto => ({
  id,
  board_id: 'board-1',
  name,
  position_key: id,
  color: 'green',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const task = (id: string, readableId: string, columnId: string): TaskSummaryDto => ({
  id,
  readable_id: readableId,
  board_id: 'board-1',
  column_id: columnId,
  board_name: 'Board',
  column_name: 'Todo',
  title: `Task ${id}`,
  priority: null,
  subtask_count: 0,
  labels: [],
  assignees: [],
  updated_at: '2026-01-01T00:00:00Z',
});

function seedBoard(): void {
  const boards = useBoardsStore();
  boards.columns = [column('c1', 'Todo'), column('c2', 'Done')];
  boards._setTasksForTest({
    c1: [task('t1', 'ATL-1', 'c1')],
    c2: [task('t2', 'ATL-2', 'c2')],
  });
}

describe('TaskListView drag auto-scroll', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('scrolls the list container when dragging near an edge', () => {
    seedBoard();

    const wrapper = mount(TaskListView, {
      props: { ws: 'ws', selectedReadableId: null },
      global: {
        stubs: {
          TaskListRow: true,
          ConfirmDialog: true,
          ContextMenu: true,
          Icon: true,
          PromptDialog: true,
        },
      },
    });

    const scrollContainer = wrapper.find('.atl-tl-scroll').element as HTMLElement;
    Object.defineProperties(scrollContainer, {
      clientHeight: { configurable: true, value: 100 },
      clientWidth: { configurable: true, value: 100 },
      scrollHeight: { configurable: true, value: 300 },
      scrollWidth: { configurable: true, value: 100 },
    });
    scrollContainer.scrollTop = 100;
    scrollContainer.getBoundingClientRect = () => ({
      x: 0,
      y: 0,
      top: 0,
      left: 0,
      right: 100,
      bottom: 100,
      width: 100,
      height: 100,
      toJSON: () => ({}),
    });

    const draggables = wrapper.findAllComponents({ name: 'VueDraggable' });
    expect(draggables).toHaveLength(2);
    for (const draggable of draggables) {
      expect(draggable.props()).toMatchObject({
        scroll: true,
        scrollSensitivity: 60,
        scrollSpeed: 14,
        bubbleScroll: true,
        forceAutoScrollFallback: true,
      });
    }

    const onMove = draggables[0]?.props('onMove') as (
      event: { to: HTMLElement },
      originalEvent: Event,
    ) => void;
    onMove(
      { to: draggables[0]?.element as HTMLElement },
      new MouseEvent('mousemove', { clientX: 50, clientY: 96 }),
    );

    expect(scrollContainer.scrollTop).toBe(114);
  });
});
