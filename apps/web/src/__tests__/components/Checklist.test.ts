import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { createMemoryHistory, createRouter } from 'vue-router';
import Checklist from '@/components/tareas/Checklist.vue';
import type { ChecklistItemDto } from '@/stores/taskDetail';

const router = createRouter({
  history: createMemoryHistory(),
  routes: [{ path: '/t/task/:readableId', name: 'task-detail', component: { template: '<div />' } }],
});

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

    expect(wrapper.text()).toContain('Checklist · 1 / 3');
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

  it('links the promoted readable id to its task instead of a promote button', () => {
    const wrapper = mount(Checklist, {
      props: { items: [item('a', 'One', false, 'ATL-99')], columns },
      global: { plugins: [router] },
    });

    const link = wrapper.get('a.atl-checklist-promoted');
    expect(link.text()).toBe('ATL-99');
    expect(link.attributes('href')).toBe('/t/task/ATL-99');
    expect(wrapper.find('button[aria-label="Promote to task"]').exists()).toBe(false);
  });

  it('emits remove with the item id when the delete button is clicked', async () => {
    const wrapper = mount(Checklist, { props: { items: [item('a', 'One', false)], columns } });

    await wrapper.get('button[aria-label="Delete item"]').trigger('click');

    expect(wrapper.emitted('remove')).toEqual([['a']]);
  });

  it('does not show the promote button when no columns are available', () => {
    const wrapper = mount(Checklist, { props: { items: [item('a', 'One', false)], columns: [] } });

    expect(wrapper.find('button[aria-label="Promote to task"]').exists()).toBe(false);
  });

  it('swaps the title for an input and emits edit with (id, title) on enter', async () => {
    const wrapper = mount(Checklist, { props: { items: [item('a', 'One', false)], columns } });

    await wrapper.get('button[aria-label="Edit item"]').trigger('click');

    const input = wrapper.get('input[aria-label="Edit item"]');
    await input.setValue('One edited');
    await input.trigger('keydown.enter');

    expect(wrapper.emitted('edit')).toEqual([['a', 'One edited']]);
  });

  it('begins editing on a double-click of the title', async () => {
    const wrapper = mount(Checklist, { props: { items: [item('a', 'One', false)], columns } });

    await wrapper.get('[data-checklist-item="a"] span').trigger('dblclick');

    expect(wrapper.find('input[aria-label="Edit item"]').exists()).toBe(true);
  });

  it('cancels the edit on escape without emitting', async () => {
    const wrapper = mount(Checklist, { props: { items: [item('a', 'One', false)], columns } });

    await wrapper.get('button[aria-label="Edit item"]').trigger('click');
    const input = wrapper.get('input[aria-label="Edit item"]');
    await input.setValue('discard me');
    await input.trigger('keydown.esc');

    expect(wrapper.emitted('edit')).toBeUndefined();
    expect(wrapper.find('input[aria-label="Edit item"]').exists()).toBe(false);
  });

  it('does not emit edit when the title is cleared to empty', async () => {
    const wrapper = mount(Checklist, { props: { items: [item('a', 'One', false)], columns } });

    await wrapper.get('button[aria-label="Edit item"]').trigger('click');
    const input = wrapper.get('input[aria-label="Edit item"]');
    await input.setValue('   ');
    await input.trigger('keydown.enter');

    expect(wrapper.emitted('edit')).toBeUndefined();
  });
});
