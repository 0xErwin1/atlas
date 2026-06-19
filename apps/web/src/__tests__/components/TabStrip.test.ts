import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import TabStrip, { type Tab } from '@/components/ui/TabStrip.vue';

const tabs: Tab[] = [
  { id: 'a', name: 'A', icon: 'file', active: false },
  { id: 'b', name: 'B', icon: 'file', active: true },
  { id: 'c', name: 'C', icon: 'file', active: false },
];

function mountStrip() {
  return mount(TabStrip, {
    props: { tabs, closable: true },
    global: { stubs: { teleport: true } },
  });
}

async function menuItemsFor(wrapper: ReturnType<typeof mountStrip>, tabName: string) {
  const tab = wrapper.findAll('[role="tab"]').find((t) => t.text().includes(tabName));
  if (tab === undefined) throw new Error('tab not found');
  await tab.trigger('contextmenu');
  await wrapper.vm.$nextTick();
  return wrapper.findComponent(ContextMenu).props('items') as Array<{
    label?: string;
    disabled?: boolean;
    action?: () => void;
  }>;
}

describe('TabStrip context menu', () => {
  it('offers close / close others / close to the right / close all', async () => {
    const items = (await menuItemsFor(mountStrip(), 'B'))
      .map((i) => i.label)
      .filter((l): l is string => typeof l === 'string');

    expect(items).toEqual(['Close', 'Close others', 'Close to the right', 'Close all']);
  });

  it('emits close-others with the tab id', async () => {
    const wrapper = mountStrip();
    const items = await menuItemsFor(wrapper, 'B');
    items.find((i) => i.label === 'Close others')?.action?.();

    expect(wrapper.emitted('close-others')).toEqual([['b']]);
  });

  it('emits close-all', async () => {
    const wrapper = mountStrip();
    const items = await menuItemsFor(wrapper, 'A');
    items.find((i) => i.label === 'Close all')?.action?.();

    expect(wrapper.emitted('close-all')).toEqual([[]]);
  });

  it('disables "close to the right" on the last tab', async () => {
    const items = await menuItemsFor(mountStrip(), 'C');
    expect(items.find((i) => i.label === 'Close to the right')?.disabled).toBe(true);
  });
});
