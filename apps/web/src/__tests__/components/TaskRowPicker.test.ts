import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import TaskRowPicker, { type PickerOption } from '@/components/tareas/TaskRowPicker.vue';

const OPTIONS: PickerOption[] = [
  { value: 'todo', label: 'Todo', color: '#000', active: true },
  { value: 'doing', label: 'Doing', color: '#111' },
  { value: '', label: 'Clear', icon: 'x', muted: true },
];

function mountPicker(open: boolean) {
  return mount(TaskRowPicker, {
    props: { options: OPTIONS, open },
    slots: { trigger: '<span class="probe">marker</span>' },
    global: { stubs: { teleport: true, Icon: true } },
  });
}

describe('TaskRowPicker', () => {
  it('renders the trigger slot and no options while closed', () => {
    const wrapper = mountPicker(false);

    expect(wrapper.find('.probe').exists()).toBe(true);
    expect(wrapper.findAll('.atl-rp-item')).toHaveLength(0);
  });

  it('lists every option when open', () => {
    const wrapper = mountPicker(true);

    const items = wrapper.findAll('.atl-rp-item');
    expect(items).toHaveLength(3);
    expect(items[2]?.classes()).toContain('muted');
  });

  it('emits the option value on pick and requests close', async () => {
    const wrapper = mountPicker(true);

    await wrapper.findAll('.atl-rp-item')[1]?.trigger('click');

    expect(wrapper.emitted('pick')?.[0]).toEqual(['doing']);
    expect(wrapper.emitted('update:open')?.at(-1)).toEqual([false]);
  });

  it('opens via the trigger and stops click propagation', async () => {
    const wrapper = mountPicker(false);

    await wrapper.find('.atl-rp-trigger').trigger('click');

    expect(wrapper.emitted('update:open')?.[0]).toEqual([true]);
  });
});
