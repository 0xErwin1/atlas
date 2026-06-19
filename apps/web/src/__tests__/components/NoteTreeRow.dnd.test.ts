import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import NoteTreeRow from '@/components/notas/NoteTreeRow.vue';
import { docKey, folderKey, type TreeFolder } from '@/lib/notesTree';
import { useTreeSelection } from '@/stores/treeSelection';

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

function dropWith(...nodes: unknown[]) {
  return { dataTransfer: { getData: () => JSON.stringify({ nodes }) } };
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
  it('emits move-nodes when a document is dropped on the folder', async () => {
    const wrapper = mountRow();
    const dropTarget = dropTargetOf(wrapper);

    await dropTarget.trigger('drop', dropWith({ type: 'doc', id: 'other-doc' }));

    expect(wrapper.emitted('move-nodes')).toEqual([[[{ type: 'doc', id: 'other-doc' }], 'folder-1']]);
  });

  it('emits move-nodes for every dragged node when several are dropped', async () => {
    const wrapper = mountRow();
    const dropTarget = dropTargetOf(wrapper);

    await dropTarget.trigger(
      'drop',
      dropWith({ type: 'doc', id: 'a' }, { type: 'folder', id: 'other-folder' }),
    );

    expect(wrapper.emitted('move-nodes')).toEqual([
      [
        [
          { type: 'doc', id: 'a' },
          { type: 'folder', id: 'other-folder' },
        ],
        'folder-1',
      ],
    ]);
  });

  it('drops the target folder itself from a multi-drag onto that folder', async () => {
    const wrapper = mountRow();
    const dropTarget = dropTargetOf(wrapper);

    await dropTarget.trigger('drop', dropWith({ type: 'folder', id: 'folder-1' }, { type: 'doc', id: 'a' }));

    expect(wrapper.emitted('move-nodes')).toEqual([[[{ type: 'doc', id: 'a' }], 'folder-1']]);
  });

  it('emits nothing when only the target folder itself is dropped on it', async () => {
    const wrapper = mountRow();
    const dropTarget = dropTargetOf(wrapper);

    await dropTarget.trigger('drop', dropWith({ type: 'folder', id: 'folder-1' }));

    expect(wrapper.emitted('move-nodes')).toBeUndefined();
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

    expect(JSON.parse(stored)).toEqual({ nodes: [{ type: 'folder', id: 'folder-1' }] });
  });

  it('dragging a selected item carries the whole selection', async () => {
    const wrapper = mountRow();
    const selection = useTreeSelection();
    selection.selectOnly(folderKey('folder-1'));
    selection.toggle(docKey('readme'));

    const dropTarget = dropTargetOf(wrapper);
    let stored = '';
    await dropTarget.trigger('dragstart', {
      dataTransfer: {
        setData: (_type: string, val: string) => {
          stored = val;
        },
        effectAllowed: '',
      },
    });

    const nodes = (JSON.parse(stored) as { nodes: Array<{ type: string; id: string }> }).nodes;
    expect(new Set(nodes.map((n) => `${n.type}:${n.id}`))).toEqual(
      new Set(['folder:folder-1', 'doc:readme']),
    );
  });
});
