import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import TaskDetailHeader from '@/components/tareas/TaskDetailHeader.vue';

function mountHeader(props: Record<string, unknown> = {}) {
  return mount(TaskDetailHeader, {
    props: { readableId: 'ATL-14', shareLabel: 'ATL-14 · task', ...props },
  });
}

describe('TaskDetailHeader', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('shows a back button and emits back when showBack is set', async () => {
    const wrapper = mountHeader({ showBack: true, showClose: false });

    const back = wrapper.find('[aria-label="Back to board"]');
    expect(back.exists()).toBe(true);
    await back.trigger('click');

    expect(wrapper.emitted('back')).toEqual([[]]);
    expect(wrapper.find('[aria-label="Close task"]').exists()).toBe(false);
  });

  it('hides the back button by default', () => {
    const wrapper = mountHeader();

    expect(wrapper.find('[aria-label="Back to board"]').exists()).toBe(false);
    expect(wrapper.find('[aria-label="Close task"]').exists()).toBe(true);
  });
});
