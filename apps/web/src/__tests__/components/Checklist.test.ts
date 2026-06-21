import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import Checklist from '@/components/tareas/Checklist.vue';
import type { ChecklistItemDto } from '@/stores/taskDetail';

const item = (id: string, title: string, checked: boolean, promoted?: string): ChecklistItemDto => ({
  id,
  task_id: 't1',
  title,
  checked,
  position_key: 'a',
  promoted_readable_id: promoted ?? null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const columns = [
  { id: 'col-1', name: 'To Do' },
  { id: 'col-2', name: 'In Progress' },
];

describe('Checklist (REQ-W22)', () => {
  it('shows the done / total progress header', () => {
    const wrapper = mount(Checklist, {
      props: { items: [item('a', 'One', true), item('b', 'Two', false), item('c', 'Three', false)], columns },
    });

    expect(wrapper.text()).toContain('Sub-tasks · 1 / 3');
  });

  it('emits toggle with the item id when the checkbox is clicked', async () => {
    const wrapper = mount(Checklist, { props: { items: [item('a', 'One', false)], columns } });

    await wrapper.get('button[aria-label="Check item"]').trigger('click');

    expect(wrapper.emitted('toggle')).toEqual([['a']]);
  });

  it('opens a column picker when the promote button is clicked', async () => {
    const wrapper = mount(Checklist, { props: { items: [item('a', 'One', false)], columns } });

    await wrapper.get('button[aria-label="Promote to task"]').trigger('click');

    expect(wrapper.text()).toContain('To Do');
    expect(wrapper.text()).toContain('In Progress');
  });

  it('emits promote with itemId and columnId when a column is picked', async () => {
    const wrapper = mount(Checklist, { props: { items: [item('a', 'One', false)], columns } });

    await wrapper.get('button[aria-label="Promote to task"]').trigger('click');
    await wrapper.get('button[data-column-id="col-1"]').trigger('click');

    expect(wrapper.emitted('promote')).toEqual([['a', 'col-1']]);
  });

  it('renders the promoted readable id instead of a promote button once promoted', () => {
    const wrapper = mount(Checklist, { props: { items: [item('a', 'One', false, 'ATL-99')], columns } });

    expect(wrapper.text()).toContain('ATL-99');
    expect(wrapper.find('button[aria-label="Promote to task"]').exists()).toBe(false);
  });

  it('emits remove with the item id when the delete button is clicked', async () => {
    const wrapper = mount(Checklist, { props: { items: [item('a', 'One', false)], columns } });

    await wrapper.get('button[aria-label="Delete sub-task"]').trigger('click');

    expect(wrapper.emitted('remove')).toEqual([['a']]);
  });

  it('does not show the promote button when no columns are available', () => {
    const wrapper = mount(Checklist, { props: { items: [item('a', 'One', false)], columns: [] } });

    expect(wrapper.find('button[aria-label="Promote to task"]').exists()).toBe(false);
  });
});
