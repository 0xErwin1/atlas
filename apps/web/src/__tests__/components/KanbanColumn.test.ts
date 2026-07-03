import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';

vi.mock('vue-draggable-plus', () => ({
  VueDraggable: {
    name: 'VueDraggable',
    props: ['modelValue'],
    template: '<div class="vdp-stub"><slot /></div>',
  },
}));

import KanbanColumn from '@/components/tareas/KanbanColumn.vue';
import { resolveColumnSwatchId } from '@/lib/columnColor';
import type { ColumnDto } from '@/stores/boards';

interface MenuItem {
  label?: string;
  disabled?: boolean;
  action?: () => void;
}

const column = (id: string, name: string, color: string | null = 'green'): ColumnDto => ({
  id,
  board_id: 'b1',
  name,
  position_key: 'm',
  color,
  created_at: 'x',
  updated_at: 'x',
});

function mountColumn(props: Partial<InstanceType<typeof KanbanColumn>['$props']> = {}) {
  return mount(KanbanColumn, {
    props: { column: column('c1', 'Todo'), tasks: [], ...props },
    global: {
      stubs: {
        TaskCard: true,
        ContextMenu: true,
        ColorPicker: true,
        Btn: { template: '<button><slot /></button>' },
        Icon: true,
      },
    },
  });
}

function menuItems(wrapper: ReturnType<typeof mountColumn>): MenuItem[] {
  return (wrapper.vm as unknown as { menuItems: MenuItem[] }).menuItems;
}

describe('KanbanColumn editing', () => {
  it('double-clicking the name enters edit mode and Enter emits save-column with the draft', async () => {
    const col = column('c1', 'Todo');
    const wrapper = mountColumn({ column: col });

    await wrapper.find('span[title="Double-click to edit"]').trigger('dblclick');

    const input = wrapper.find('input.atl-col-rename');
    expect(input.exists()).toBe(true);

    await input.setValue('Backlog');
    await input.trigger('keydown.enter');

    const saved = wrapper.emitted('save-column');
    expect(saved).toBeTruthy();
    expect(saved?.[0]).toEqual([col, { name: 'Backlog', color: resolveColumnSwatchId(col) }]);
  });

  it('Escape leaves edit mode without emitting', async () => {
    const wrapper = mountColumn();

    await wrapper.find('span[title="Double-click to edit"]').trigger('dblclick');
    await wrapper.find('input.atl-col-rename').trigger('keydown.esc');

    expect(wrapper.find('input.atl-col-rename').exists()).toBe(false);
    expect(wrapper.emitted('save-column')).toBeUndefined();
  });

  it('the menu emits move-column and delete-column', () => {
    const col = column('c1', 'Todo');
    const wrapper = mountColumn({ column: col });

    const items = menuItems(wrapper);
    items.find((i) => i.label === 'Move right')?.action?.();
    items.find((i) => i.label === 'Delete status')?.action?.();

    expect(wrapper.emitted('move-column')?.[0]).toEqual([col, 1]);
    expect(wrapper.emitted('delete-column')?.[0]).toEqual([col]);
  });

  it('disables Move left on the first column and Move right on the last', () => {
    const first = menuItems(mountColumn({ isFirst: true }));
    expect(first.find((i) => i.label === 'Move left')?.disabled).toBe(true);
    expect(first.find((i) => i.label === 'Move right')?.disabled).toBe(false);

    const last = menuItems(mountColumn({ isLast: true }));
    expect(last.find((i) => i.label === 'Move right')?.disabled).toBe(true);
  });
});
