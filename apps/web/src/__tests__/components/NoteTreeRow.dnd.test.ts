import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import NoteTreeRow from '@/components/notas/NoteTreeRow.vue';
import type { TreeFolder } from '@/lib/notesTree';

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET: vi.fn().mockResolvedValue({ data: { state: {} } }), PUT: vi.fn() },
}));

beforeEach(() => {
  setActivePinia(createPinia());
});

function folder(): TreeFolder {
  return {
    kind: 'folder',
    id: 'folder-1',
    name: 'Specs',
    folders: [],
    docs: [{ kind: 'doc', id: 'd1', slug: 'readme', title: 'Readme' }],
  };
}

function dropWith(node: unknown) {
  return { dataTransfer: { getData: () => JSON.stringify(node) } };
}

function mountRow() {
  return mount(NoteTreeRow, {
    props: { folder: folder(), depth: 0, activeSlug: null },
  });
}

function dropTargetOf(wrapper: ReturnType<typeof mountRow>) {
  const el = wrapper.findAll('.tree-dnd')[0];
  if (el === undefined) throw new Error('no drop target rendered');
  return el;
}

describe('NoteTreeRow drag-and-drop', () => {
  it('emits move-doc when a document is dropped on the folder', async () => {
    const wrapper = mountRow();
    const dropTarget = dropTargetOf(wrapper);

    await dropTarget.trigger('drop', dropWith({ type: 'doc', id: 'other-doc' }));

    expect(wrapper.emitted('move-doc')).toEqual([['other-doc', 'folder-1']]);
  });

  it('emits move-folder when another folder is dropped on it', async () => {
    const wrapper = mountRow();
    const dropTarget = dropTargetOf(wrapper);

    await dropTarget.trigger('drop', dropWith({ type: 'folder', id: 'other-folder' }));

    expect(wrapper.emitted('move-folder')).toEqual([['other-folder', 'folder-1']]);
  });

  it('ignores a folder dropped onto itself', async () => {
    const wrapper = mountRow();
    const dropTarget = dropTargetOf(wrapper);

    await dropTarget.trigger('drop', dropWith({ type: 'folder', id: 'folder-1' }));

    expect(wrapper.emitted('move-folder')).toBeUndefined();
  });

  it('writes the drag payload on dragstart', async () => {
    const wrapper = mountRow();
    const dropTarget = dropTargetOf(wrapper);
    let stored = '';
    const dataTransfer = {
      setData: (_type: string, val: string) => {
        stored = val;
      },
      effectAllowed: '',
    };

    await dropTarget.trigger('dragstart', { dataTransfer });

    expect(JSON.parse(stored)).toEqual({ type: 'folder', id: 'folder-1' });
  });
});
