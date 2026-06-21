import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import MultiSelect, { type MultiSelectOption } from '@/components/ui/MultiSelect.vue';

const OPTIONS: MultiSelectOption[] = [
  { value: 'note', label: 'Notes', icon: 'notes' },
  { value: 'task', label: 'Tasks', icon: 'square-kanban' },
  { value: 'doc', label: 'Docs', icon: 'file-text', disabled: true },
  { value: 'comment', label: 'Comments', icon: 'message-square', disabled: true },
];

async function openPanel(wrapper: ReturnType<typeof mount>): Promise<void> {
  await wrapper.get('.atl-popover > div').trigger('click');
}

function optionRow(wrapper: ReturnType<typeof mount>, label: string) {
  const row = wrapper.findAll('[role="option"]').find((r) => r.text().includes(label));
  if (!row) throw new Error(`option row "${label}" not found`);
  return row;
}

describe('MultiSelect disabled options (SE7, SE8)', () => {
  it('renders a disabled option visibly and marks it aria-disabled (SE7)', async () => {
    const wrapper = mount(MultiSelect, {
      props: { options: OPTIONS, modelValue: [] },
    });
    await openPanel(wrapper);

    const docs = optionRow(wrapper, 'Docs');
    expect(docs.exists()).toBe(true);
    expect(docs.attributes('aria-disabled')).toBe('true');
  });

  it('does not mark an enabled option aria-disabled (SE7)', async () => {
    const wrapper = mount(MultiSelect, {
      props: { options: OPTIONS, modelValue: [] },
    });
    await openPanel(wrapper);

    const notes = optionRow(wrapper, 'Notes');
    expect(notes.attributes('aria-disabled')).toBeUndefined();
  });

  it('clicking a disabled option does not emit a model update (SE8)', async () => {
    const wrapper = mount(MultiSelect, {
      props: { options: OPTIONS, modelValue: [] },
    });
    await openPanel(wrapper);

    await optionRow(wrapper, 'Docs').trigger('click');
    await optionRow(wrapper, 'Comments').trigger('click');

    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
  });

  it('clicking an enabled option still toggles it on (SE8 control)', async () => {
    const wrapper = mount(MultiSelect, {
      props: { options: OPTIONS, modelValue: [] },
    });
    await openPanel(wrapper);

    await optionRow(wrapper, 'Notes').trigger('click');

    const emitted = wrapper.emitted('update:modelValue');
    expect(emitted).toBeTruthy();
    expect(emitted?.[0]).toEqual([['note']]);
  });

  it('clicking an enabled option toggles it off again (SE8 control)', async () => {
    const wrapper = mount(MultiSelect, {
      props: { options: OPTIONS, modelValue: ['task'] },
    });
    await openPanel(wrapper);

    await optionRow(wrapper, 'Tasks').trigger('click');

    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual([[]]);
  });
});
