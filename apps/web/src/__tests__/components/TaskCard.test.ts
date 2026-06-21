import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import TaskCard from '@/components/tareas/TaskCard.vue';
import Avatar from '@/components/ui/Avatar.vue';
import type { TaskSummaryDto } from '@/stores/boards';

beforeEach(() => {
  setActivePinia(createPinia());
});

const task = (overrides: Partial<TaskSummaryDto> = {}): TaskSummaryDto => ({
  id: 't1',
  readable_id: 'AB-1',
  column_id: 'col-1',
  board_name: 'Board',
  column_name: 'Todo',
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

  it('renders a chip per label', () => {
    const wrapper = mount(TaskCard, { props: { task: task({ labels: ['shell', 'M1'] }) } });
    const text = wrapper.text();
    expect(text).toContain('shell');
    expect(text).toContain('M1');
  });

  it('renders an avatar per assignee, agent-styled for api keys', () => {
    const wrapper = mount(TaskCard, {
      props: {
        task: task({
          assignees: [
            { type: 'user', id: 'u1', display_name: 'Mara K' },
            { type: 'api_key', id: 'k1', display_name: 'CI Bot' },
          ],
        }),
      },
    });
    const avatars = wrapper.findAllComponents(Avatar);
    expect(avatars).toHaveLength(2);
    expect(avatars[1]?.props('agent')).toBe(true);
  });

  it('renders no avatar when there are no assignees', () => {
    const wrapper = mount(TaskCard, { props: { task: task() } });
    expect(wrapper.findComponent(Avatar).exists()).toBe(false);
  });
});
