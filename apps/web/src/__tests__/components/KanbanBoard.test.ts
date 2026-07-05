import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { POST } = vi.hoisted(() => ({ POST: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { POST },
}));

// Stub vue-draggable-plus: render the slot, expose the SortableJS drop events.
vi.mock('vue-draggable-plus', () => ({
  VueDraggable: {
    name: 'VueDraggable',
    props: ['modelValue'],
    emits: ['add', 'update'],
    template: '<div class="vdp-stub"><slot /></div>',
  },
}));

import KanbanBoard from '@/components/tareas/KanbanBoard.vue';
import KanbanColumn from '@/components/tareas/KanbanColumn.vue';
import { type ColumnDto, type TaskSummaryDto, useBoardsStore } from '@/stores/boards';
import { useUiStore } from '@/stores/ui';

const column = (id: string, name: string, pos: string): ColumnDto => ({
  id,
  board_id: 'board-1',
  name,
  position_key: pos,
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
  updated_at: '2026-01-01T00:00:00Z',
});

const taskDto = (id: string, readableId: string, columnId: string) => ({
  id,
  readable_id: readableId,
  column_id: columnId,
  board_id: 'board-1',
  title: `Task ${id}`,
  description: '',
  priority: 'high',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-02T00:00:00Z',
  workspace_id: 'ws',
  project_id: 'proj',
  created_by: { id: 'u1', type: 'user', display_name: 'User' },
  labels: [],
});

function seedBoard() {
  const store = useBoardsStore();
  store.columns = [column('c1', 'Backlog', 'a'), column('c2', 'In progress', 'b')];
  store._setTasksForTest({
    c1: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1')],
    c2: [task('t3', 'ATL-3', 'c2')],
  });
  return store;
}

describe('KanbanBoard drag-and-drop wiring', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('renders a column per board column with the correct task count', () => {
    seedBoard();
    const wrapper = mount(KanbanBoard, { props: { ws: 'ws' } });

    const columns = wrapper.findAllComponents(KanbanColumn);
    expect(columns).toHaveLength(2);
    expect(wrapper.text()).toContain('Backlog');
    expect(wrapper.text()).toContain('In progress');
    expect(wrapper.text()).toContain('ATL-1');
  });

  it('translates a cross-column drop into a move POST with the destination column and index', async () => {
    seedBoard();
    POST.mockResolvedValueOnce({ data: taskDto('t1', 'ATL-1', 'c2'), error: undefined });

    const wrapper = mount(KanbanBoard, { props: { ws: 'ws' } });
    const inProgress = wrapper.findAllComponents(KanbanColumn)[1];

    // Simulate SortableJS firing `added` on the destination column.
    inProgress?.vm.$emit('drop', 'ATL-1', 'c2', 1);
    await flushPromises();

    expect(POST).toHaveBeenCalledOnce();
    const [path, opts] = POST.mock.calls[0] as [
      string,
      { params: { path: { readable_id: string } }; body: { column_id: string } },
    ];
    expect(path).toBe('/v1/workspaces/{ws}/tasks/{readable_id}/move');
    expect(opts.params.path.readable_id).toBe('ATL-1');
    expect(opts.body.column_id).toBe('c2');
  });

  it('surfaces the API hint via a banner when the move fails', async () => {
    seedBoard();
    POST.mockResolvedValueOnce({
      data: undefined,
      error: { type: 'urn:atlas:error:forbidden', hint: 'You cannot move this task', status: 403 },
    });

    const ui = useUiStore();
    const spy = vi.spyOn(ui, 'showBanner');

    const wrapper = mount(KanbanBoard, { props: { ws: 'ws' } });
    const inProgress = wrapper.findAllComponents(KanbanColumn)[1];

    inProgress?.vm.$emit('drop', 'ATL-1', 'c2', 0);
    await flushPromises();

    expect(spy).toHaveBeenCalledWith('You cannot move this task', 'error');
  });

  it('a SortableJS @add event resolves to a drop emit through resolveDropTarget', async () => {
    seedBoard();
    POST.mockResolvedValueOnce({ data: taskDto('t3', 'ATL-3', 'c1'), error: undefined });

    const wrapper = mount(KanbanBoard, { props: { ws: 'ws' } });
    const backlog = wrapper.findAllComponents(KanbanColumn)[0];

    // Fire the SortableJS `add` event on the inner draggable stub, carrying the
    // dragged DOM node with its data-readable-id.
    const draggable = backlog?.findComponent({ name: 'VueDraggable' });
    draggable?.vm.$emit('add', { item: { dataset: { readableId: 'ATL-3' } }, newIndex: 0 });
    await flushPromises();

    expect(POST).toHaveBeenCalledOnce();
    const [, opts] = POST.mock.calls[0] as [string, { body: { column_id: string } }];
    expect(opts.body.column_id).toBe('c1');
  });
});
