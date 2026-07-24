import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: vi.fn() }),
}));

import BoardViewMenu from '@/components/tareas/BoardViewMenu.vue';
import { useUiStore } from '@/stores/ui';

describe('BoardViewMenu', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('uses the shared toolbar dropdown trigger treatment', () => {
    useUiStore().setTaskView('list');
    const wrapper = mount(BoardViewMenu, {
      global: {
        stubs: {
          Icon: { props: ['name'], template: '<i :data-icon="name" />' },
          NewViewDialog: true,
          Popover: {
            template: '<div><slot name="trigger" :open="false" :toggle="() => {}" /></div>',
          },
        },
      },
    });

    const trigger = wrapper.get('button[aria-haspopup="menu"]');

    expect(trigger.classes()).toContain('atl-gbtn');
    expect(trigger.classes()).not.toContain('atl-dd');
    expect(trigger.attributes('style')).toBeUndefined();
    expect(trigger.attributes('title')).toBe('View: List');
    expect(trigger.text()).toBe('List');
    expect(trigger.find('[data-icon="chevron-down"]').exists()).toBe(false);
  });
});
