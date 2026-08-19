import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import KanbanBoard from '@/components/tareas/KanbanBoard.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import { type ColumnDto, type TaskSummaryDto, useBoardsStore } from '@/stores/boards';

vi.mock('vue-draggable-plus', () => ({
  VueDraggable: {
    name: 'VueDraggable',
    props: ['modelValue'],
    template: '<div class="vdp-stub"><slot /></div>',
  },
}));

/**
 * The board's scroll handler exists only to drive the mobile dot strip. It reads
 * scrollLeft/scrollWidth/clientWidth, which forces a reflow, so it must not run
 * at all on a viewport that never renders the strip.
 */

const column = (id: string, name: string, pos: string): ColumnDto => ({
  id,
  board_id: 'board-1',
  name,
  position_key: pos,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const task = (id: string, columnId: string): TaskSummaryDto => ({
  id,
  readable_id: `ATL-${id}`,
  board_id: 'board-1',
  column_id: columnId,
  board_name: 'Board',
  column_name: 'Todo',
  title: `Task ${id}`,
  priority: null,
  subtask_count: 0,
  updated_at: '2026-01-01T00:00:00Z',
});

let frames: FrameRequestCallback[] = [];

function runFrames(): void {
  const pending = frames;
  frames = [];
  for (const frame of pending) frame(0);
}

function setViewportWidth(width: number): void {
  useBreakpoint().viewportWidth.value = width;
}

function seedBoard() {
  const store = useBoardsStore();
  store.columns = [column('c1', 'Backlog', 'a'), column('c2', 'In progress', 'b')];
  store._setTasksForTest({ c1: [task('t1', 'c1')], c2: [task('t2', 'c2')] });
  return store;
}

function spyOnScrollMetrics(el: Element): ReturnType<typeof vi.fn> {
  const reads = vi.fn(() => 1200);
  Object.defineProperty(el, 'scrollWidth', { get: reads, configurable: true });
  return reads;
}

describe('KanbanBoard scroll handling', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    frames = [];
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => frames.push(cb));
    vi.stubGlobal('cancelAnimationFrame', () => {
      frames = [];
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    setViewportWidth(1024);
  });

  it('reads no layout metrics on a desktop viewport, where no dot strip renders', async () => {
    setViewportWidth(1280);
    seedBoard();

    const wrapper = mount(KanbanBoard, { props: { ws: 'ws' } });
    const scroller = wrapper.get('.overflow-x-auto');
    const reads = spyOnScrollMetrics(scroller.element);

    await scroller.trigger('scroll');
    runFrames();

    expect(reads).not.toHaveBeenCalled();
  });

  it('coalesces the mobile dot-strip reads into one animation frame', async () => {
    setViewportWidth(390);
    seedBoard();

    const wrapper = mount(KanbanBoard, { props: { ws: 'ws' } });
    await nextTick();

    const scroller = wrapper.get('.overflow-x-auto');
    const reads = spyOnScrollMetrics(scroller.element);

    await scroller.trigger('scroll');
    await scroller.trigger('scroll');
    await scroller.trigger('scroll');

    expect(reads).not.toHaveBeenCalled();
    expect(frames).toHaveLength(1);

    runFrames();

    expect(reads).toHaveBeenCalledTimes(1);
  });

  it('registers the board scroll listener passively', () => {
    setViewportWidth(1280);
    seedBoard();

    const listeners = vi.spyOn(Element.prototype, 'addEventListener');
    mount(KanbanBoard, { props: { ws: 'ws' } });

    const scroll = listeners.mock.calls.find(([type]) => type === 'scroll');
    expect(scroll?.[2]).toMatchObject({ passive: true });

    listeners.mockRestore();
  });
});
