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
    boards: [],
  };
}

function folderWithBoard(): TreeFolder {
  return {
    kind: 'folder',
    id: 'folder-1',
    name: 'Specs',
    folders: [],
    docs: [],
    boards: [{ kind: 'board', id: 'b1', name: 'Roadmap', taskCount: 12 }],
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
  it('opens the create menu from the folder add trigger', async () => {
    const wrapper = mountRow();
    const folderTrigger = wrapper.get('button[aria-label="Add page or folder"]');

    expect(folderTrigger.find('.lucide-plus').exists()).toBe(true);
    await wrapper.get('.atl-row').trigger('click');
    expect(wrapper.get('button[aria-label="More actions"]')).toBeTruthy();

    await folderTrigger.trigger('click');

    const menu = document.body.querySelector('[role="menu"]');
    expect(menu?.textContent).toContain('New page');
    expect(menu?.textContent).toContain('New folder');

    wrapper.unmount();
  });

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

  it('keeps the folder drag payload while exposing its hierarchy level', async () => {
    const wrapper = mountRow();
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

    expect(wrapper.get('[role="treeitem"][aria-label="Folder: Specs"]').attributes('aria-level')).toBe('1');
    expect(JSON.parse(stored)).toEqual({ nodes: [{ type: 'folder', id: 'folder-1' }] });
  });

  it('renders a nested board row with its counter and emits select-board on click', async () => {
    const wrapper = mount(NoteTreeRow, {
      props: { folder: folderWithBoard(), depth: 0, activeSlug: null },
    });
    await wrapper.get('.atl-row').trigger('click');

    expect(wrapper.text()).toContain('Roadmap');
    expect(wrapper.text()).toContain('12');

    const boardButton = wrapper.findAll('button').find((b) => b.text().includes('Roadmap'));
    await boardButton?.trigger('click');

    expect(wrapper.emitted('select-board')?.[0]).toEqual(['b1']);
    wrapper.unmount();
  });

  it('drops a board node onto a sibling folder to trigger its move', async () => {
    const wrapper = mount(NoteTreeRow, {
      props: { folder: folderWithBoard(), depth: 0, activeSlug: null },
    });
    const dropTarget = wrapper.findAll('.tree-dnd')[0];

    await dropTarget?.trigger('drop', dropWith({ type: 'board', id: 'other-board' }));

    expect(wrapper.emitted('move-nodes')).toEqual([[[{ type: 'board', id: 'other-board' }], 'folder-1']]);
    wrapper.unmount();
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
