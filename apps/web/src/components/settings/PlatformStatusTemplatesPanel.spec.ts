import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import PlatformStatusTemplatesPanel from '@/components/settings/PlatformStatusTemplatesPanel.vue';
import { usePlatformStatusTemplatesStore } from '@/stores/platformStatusTemplates';

function tpl(over: Record<string, unknown> = {}) {
  return {
    id: 't1',
    name: 'Todo',
    color: null,
    position_key: 'a',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...over,
  };
}

function setup() {
  const store = usePlatformStatusTemplatesStore();
  vi.spyOn(store, 'load').mockResolvedValue(undefined);
  store.templates = [
    tpl({ id: 't1', name: 'Todo', position_key: 'a' }),
    tpl({ id: 't2', name: 'Doing', position_key: 'b' }),
  ] as never;

  return { store };
}

function actionIn(wrapper: ReturnType<typeof mount>, rowIndex: number, title: string) {
  const row = wrapper.findAll('.atl-status-row')[rowIndex];
  if (row === undefined) throw new Error(`missing row ${rowIndex}`);

  const action = row.findAll('button').find((b) => b.attributes('title') === title);
  if (action === undefined) throw new Error(`missing action ${title}`);
  return action;
}

describe('PlatformStatusTemplatesPanel', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('renders one row per Atlas default status and no apply-to-board section', async () => {
    setup();

    const wrapper = mount(PlatformStatusTemplatesPanel);
    await wrapper.vm.$nextTick();

    const rows = wrapper.findAll('.atl-status-row');
    expect(rows).toHaveLength(2);
    expect(rows[0]?.text()).toContain('Todo');
    expect(wrapper.find('.atl-apply-section').exists()).toBe(false);
  });

  it('saving an edit calls update without any workspace argument', async () => {
    const { store } = setup();
    const update = vi.spyOn(store, 'update').mockResolvedValue(true);

    const wrapper = mount(PlatformStatusTemplatesPanel);
    await wrapper.vm.$nextTick();

    await actionIn(wrapper, 0, 'Edit name & color').trigger('click');

    const nameInput = wrapper.find('.atl-status-row.editing .atl-status-rename');
    await nameInput.setValue('Backlog');
    await nameInput.trigger('keydown.enter');

    expect(update).toHaveBeenCalledWith('t1', { name: 'Backlog' });
  });

  it('moving a row down asks the store for the position after its next sibling', async () => {
    const { store } = setup();
    const move = vi.spyOn(store, 'move').mockResolvedValue(true);

    const wrapper = mount(PlatformStatusTemplatesPanel);
    await wrapper.vm.$nextTick();

    await actionIn(wrapper, 0, 'Move down').trigger('click');

    expect(move).toHaveBeenCalledWith('t1', { before: 'b', after: null });
  });

  it('confirming a delete calls remove for that row', async () => {
    const { store } = setup();
    const remove = vi.spyOn(store, 'remove').mockResolvedValue(true);

    const wrapper = mount(PlatformStatusTemplatesPanel);
    await wrapper.vm.$nextTick();

    await actionIn(wrapper, 1, 'Delete status').trigger('click');

    // ConfirmDialog teleports to <body>, so its nodes live outside the wrapper.
    const confirm = document.body.querySelector('[data-test="confirm"]') as HTMLButtonElement | null;
    if (confirm === null) throw new Error('missing confirm button');
    confirm.click();
    await flushPromises();

    expect(remove).toHaveBeenCalledWith('t2');
  });
});
