import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import TaskDetailHeader from '@/components/tareas/TaskDetailHeader.vue';
import { resetPlatformTransportForTest, setPlatformTransport } from '@/platform/transport';
import { useUiStore } from '@/stores/ui';
import { fakePlatformTransport } from '../helpers/platformTransport';

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

  afterEach(() => {
    resetPlatformTransportForTest();
  });

  it('shows a back button and emits back when showBack is set', async () => {
    const wrapper = mountHeader({ showBack: true, showClose: false });

    const back = wrapper.find('[aria-label="Back"]');
    expect(back.exists()).toBe(true);
    expect(back.attributes('title')).toBe('Back');
    await back.trigger('click');

    expect(wrapper.emitted('back')).toEqual([[]]);
    expect(wrapper.find('[aria-label="Close task"]').exists()).toBe(false);
  });

  it('hides the back button by default', () => {
    const wrapper = mountHeader();

    expect(wrapper.find('[aria-label="Back"]').exists()).toBe(false);
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

  it('shows a banner and does not touch the clipboard when the public base is unknown', async () => {
    setPlatformTransport(fakePlatformTransport({ publicBase: () => '' }));
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });

    const wrapper = mountHeader();
    const copyItem = menuItems(wrapper).find((i) => i.label === 'Copy link');
    copyItem?.action?.();

    await vi.waitFor(() =>
      expect(useUiStore().banner?.message).toBe('The server address is not available yet'),
    );
    expect(writeText).not.toHaveBeenCalled();
    expect(useUiStore().banner?.type).toBe('error');
  });

  it('copies the full public URL when the base is known', async () => {
    setPlatformTransport(fakePlatformTransport());
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });

    const wrapper = mountHeader();
    const copyItem = menuItems(wrapper).find((i) => i.label === 'Copy link');
    copyItem?.action?.();

    await vi.waitFor(() => expect(writeText).toHaveBeenCalledWith('https://atlas.test/t/task/ATL-14'));
  });

  it('shows an unknown-base banner when opening in the browser without a known origin', async () => {
    setPlatformTransport(fakePlatformTransport({ publicBase: () => '' }));

    const wrapper = mountHeader();
    const openItem = menuItems(wrapper).find((i) => i.label === 'Open in new tab');
    openItem?.action?.();

    await vi.waitFor(() =>
      expect(useUiStore().banner?.message).toBe('The server address is not available yet'),
    );
  });

  it('shows a failure banner when openExternal fails', async () => {
    setPlatformTransport(fakePlatformTransport({ openExternal: vi.fn(async () => ({ error: 'blocked' })) }));

    const wrapper = mountHeader();
    const openItem = menuItems(wrapper).find((i) => i.label === 'Open in new tab');
    openItem?.action?.();

    await vi.waitFor(() => expect(useUiStore().banner?.message).toBe("The link couldn't be opened"));
  });

  it('opens the public URL through the platform transport on success', async () => {
    const openExternal = vi.fn(async () => ({}));
    setPlatformTransport(fakePlatformTransport({ openExternal }));

    const wrapper = mountHeader();
    const openItem = menuItems(wrapper).find((i) => i.label === 'Open in new tab');
    openItem?.action?.();

    await vi.waitFor(() => expect(openExternal).toHaveBeenCalledWith('https://atlas.test/t/task/ATL-14'));
  });
});
