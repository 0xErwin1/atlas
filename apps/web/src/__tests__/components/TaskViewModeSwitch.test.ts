import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import TaskViewModeSwitch from '@/components/tareas/TaskViewModeSwitch.vue';
import { useUiStore } from '@/stores/ui';

describe('TaskViewModeSwitch', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    localStorage.clear();
  });

  it('keeps the picker closed until the trigger is clicked', async () => {
    const wrapper = mount(TaskViewModeSwitch);
    expect(wrapper.find('[role="menu"]').exists()).toBe(false);

    await wrapper.find('button').trigger('click');
    expect(wrapper.find('[role="menu"]').exists()).toBe(true);
  });

  it('offers the three view modes', async () => {
    const wrapper = mount(TaskViewModeSwitch);
    await wrapper.find('button').trigger('click');

    const labels = wrapper.findAll('[role="menuitemradio"]').map((b) => b.text());
    expect(labels).toEqual(['Dialog', 'Full screen', 'Sidebar']);
  });

  it('marks the active mode as checked', async () => {
    const ui = useUiStore();
    ui.setTaskViewMode('modal');

    const wrapper = mount(TaskViewModeSwitch);
    await wrapper.find('button').trigger('click');

    const dialog = wrapper.findAll('[role="menuitemradio"]')[0];
    expect(dialog?.attributes('aria-checked')).toBe('true');
  });

  it('picking a mode updates the store, closes the picker, and emits change', async () => {
    const ui = useUiStore();
    const wrapper = mount(TaskViewModeSwitch);
    await wrapper.find('button').trigger('click');

    await wrapper.findAll('[role="menuitemradio"]')[1]?.trigger('click');

    expect(ui.taskViewMode).toBe('full');
    expect(wrapper.find('[role="menu"]').exists()).toBe(false);
    expect(wrapper.emitted('change')).toEqual([['full']]);
  });
});
