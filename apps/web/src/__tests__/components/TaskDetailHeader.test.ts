import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import { nextTick } from 'vue';
import TaskDetailHeader from '@/components/tareas/TaskDetailHeader.vue';

interface MenuItem {
  label?: string;
  danger?: boolean;
  action?: () => void;
}

const ConfirmDialogStub = {
  props: ['open'],
  emits: ['confirm', 'cancel'],
  template: '<div v-if="open"><button data-test="confirm" @click="$emit(\'confirm\')" /></div>',
};

function mountHeader(props: Record<string, unknown> = {}) {
  return mount(TaskDetailHeader, {
    props: { readableId: 'ATL-14', shareLabel: 'ATL-14 · task', ...props },
    global: { stubs: { ConfirmDialog: ConfirmDialogStub } },
  });
}

function menuItems(wrapper: ReturnType<typeof mountHeader>): MenuItem[] {
  return (wrapper.vm as unknown as { menuItems: MenuItem[] }).menuItems;
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

  it('offers a Delete task action that emits delete only after confirmation (ATL-64)', async () => {
    const wrapper = mountHeader();

    const del = menuItems(wrapper).find((i) => i.label === 'Delete task');
    expect(del?.danger).toBe(true);

    // Opening the menu item only asks for confirmation — no delete yet.
    del?.action?.();
    await nextTick();
    expect(wrapper.emitted('delete')).toBeUndefined();

    await wrapper.find('[data-test="confirm"]').trigger('click');
    expect(wrapper.emitted('delete')).toEqual([[]]);
  });
});
