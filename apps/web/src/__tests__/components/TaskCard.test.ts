import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import TaskCard from '@/components/tareas/TaskCard.vue';
import type { TaskSummaryDto } from '@/stores/boards';

const task = (overrides: Partial<TaskSummaryDto> = {}): TaskSummaryDto => ({
  id: 't1',
  readable_id: 'AB-1',
  column_id: 'col-1',
  title: 'A task',
  priority: null,
  updated_at: '2026-01-01T00:00:00Z',
  ...overrides,
});

describe('TaskCard (Linear-style selection)', () => {
  it('emits select on single click and open on double click', async () => {
    const wrapper = mount(TaskCard, { props: { task: task() } });

    await wrapper.trigger('click');
    await wrapper.trigger('dblclick');

    expect(wrapper.emitted('select')).toEqual([['AB-1']]);
    expect(wrapper.emitted('open')).toEqual([['AB-1']]);
  });

  it('uses the accent border when selected', () => {
    const plain = mount(TaskCard, { props: { task: task(), selected: false } });
    const picked = mount(TaskCard, { props: { task: task(), selected: true } });

    expect(plain.attributes('style')).toContain('var(--c-border)');
    expect(picked.attributes('style')).toContain('var(--c-primary)');
  });
});
