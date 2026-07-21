import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import NotesTree from '@/components/notas/NotesTree.vue';
import { docKey } from '@/lib/notesTree';
import { useTreeSelection } from '@/stores/treeSelection';

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET: vi.fn().mockResolvedValue({ data: { state: {} } }), PUT: vi.fn() },
}));

beforeEach(() => {
  setActivePinia(createPinia());
});

describe('NotesTree', () => {
  it('renders nested folders and docs and emits the slug on click (REQ-W14)', async () => {
    const wrapper = mount(NotesTree, {
      props: {
        projectName: 'Atlas',
        folders: [
          { id: 'f1', name: 'Specs', parent_folder_id: null },
          { id: 'f2', name: 'Drafts', parent_folder_id: 'f1' },
        ],
        docs: [
          { id: 'd1', title: 'PRD', slug: 'prd', folder_id: 'f1' },
          { id: 'd2', title: 'Root note', slug: 'root-note', folder_id: null },
        ],
        activeSlug: 'prd',
      },
    });

    expect(wrapper.text()).toContain('Specs');
    expect(wrapper.text()).toContain('Drafts');
    expect(wrapper.text()).toContain('PRD');
    expect(wrapper.text()).toContain('Root note');
    expect(wrapper.get('[role="treeitem"][aria-label="Folder: Specs"]').attributes('aria-level')).toBe('1');
    expect(wrapper.get('[role="treeitem"][aria-label="Folder: Drafts"]').attributes('aria-level')).toBe('2');
    expect(wrapper.get('[role="treeitem"][aria-label="Page: PRD"]').attributes('aria-level')).toBe('2');

    const docButton = wrapper.findAll('button').find((b) => b.text().includes('Root note'));
    expect(docButton).toBeDefined();

    await docButton?.trigger('click');
    expect(wrapper.emitted('select-doc')?.[0]).toEqual(['root-note']);
  });

  it('shows the empty state when there is nothing to show', () => {
    const wrapper = mount(NotesTree, {
      props: { projectName: 'Atlas', folders: [], docs: [], activeSlug: null },
    });

    expect(wrapper.text()).toContain('No documents yet.');
  });

  it('disables a doc with no slug so it never navigates to an invalid route', () => {
    const wrapper = mount(NotesTree, {
      props: {
        projectName: 'Atlas',
        folders: [],
        docs: [{ id: 'd1', title: 'Unslugged', slug: null, folder_id: null }],
        activeSlug: null,
      },
    });

    const docButton = wrapper.findAll('button').find((b) => b.text().includes('Unslugged'));
    expect(docButton?.attributes('disabled')).toBeDefined();
  });

  it('renders a board row with its task counter and navigates on click (Acta counter)', async () => {
    const wrapper = mount(NotesTree, {
      props: {
        projectName: 'Atlas',
        folders: [],
        docs: [],
        boards: [{ id: 'b1', name: 'Roadmap', folder_id: null, task_count: 23 }],
        activeSlug: null,
      },
    });

    expect(wrapper.text()).toContain('Roadmap');
    expect(wrapper.text()).toContain('23');

    const boardButton = wrapper.findAll('button').find((b) => b.text().includes('Roadmap'));
    await boardButton?.trigger('click');

    expect(wrapper.emitted('select-board')?.[0]).toEqual(['b1']);
  });

  it('exposes root pages and boards as distinct hierarchy items', () => {
    const wrapper = mount(NotesTree, {
      props: {
        projectName: 'Atlas',
        folders: [],
        docs: [{ id: 'd1', title: 'Planning', slug: 'planning', folder_id: null }],
        boards: [{ id: 'b1', name: 'Roadmap', folder_id: null, task_count: 23 }],
        activeSlug: null,
      },
    });

    expect(wrapper.get('[role="tree"]').attributes('aria-label')).toBe('Atlas hierarchy');
    expect(wrapper.get('[role="treeitem"][aria-label="Page: Planning"]').attributes('aria-level')).toBe('1');
    expect(wrapper.get('[role="treeitem"][aria-label="Board: Roadmap"]').attributes('aria-level')).toBe('1');
  });

  it('keeps logical hierarchy while giving every project child a 20px visual step', async () => {
    const wrapper = mount(NotesTree, {
      props: {
        projectName: 'Atlas',
        folders: [
          { id: 'f1', name: 'Root', parent_folder_id: null },
          { id: 'f2', name: 'Nested', parent_folder_id: 'f1' },
          { id: 'f3', name: 'Deep', parent_folder_id: 'f2' },
          { id: 'orphan', name: 'Orphan', parent_folder_id: 'missing' },
        ],
        docs: [
          { id: 'd1', title: 'Root page', slug: 'root-page', folder_id: null },
          { id: 'd2', title: 'Nested page', slug: 'nested-page', folder_id: 'f1' },
        ],
        boards: [
          { id: 'b1', name: 'Root board', folder_id: null, task_count: 1 },
          { id: 'b2', name: 'Nested board', folder_id: 'f1', task_count: 2 },
        ],
        activeSlug: null,
      },
    });

    const rowStyle = (label: string) =>
      wrapper.get(`[role="treeitem"][aria-label="${label}"] .atl-row`).attributes('style');

    expect(rowStyle('Folder: Root')).toContain('padding-left: 28px');
    await wrapper.get('[role="treeitem"][aria-label="Folder: Root"] .atl-row').trigger('click');
    expect(rowStyle('Folder: Nested')).toContain('padding-left: 48px');
    await wrapper.get('[role="treeitem"][aria-label="Folder: Nested"] .atl-row').trigger('click');
    expect(rowStyle('Folder: Deep')).toContain('padding-left: 68px');
    expect(rowStyle('Folder: Orphan')).toContain('padding-left: 28px');
    expect(rowStyle('Page: Root page')).toContain('padding-left: 28px');
    expect(rowStyle('Board: Root board')).toContain('padding-left: 28px');
    expect(rowStyle('Page: Nested page')).toContain('padding-left: 48px');
    expect(rowStyle('Board: Nested board')).toContain('padding-left: 48px');
    expect(wrapper.get('[role="treeitem"][aria-label="Folder: Root"]').attributes('aria-level')).toBe('1');
    expect(wrapper.get('[role="treeitem"][aria-label="Folder: Nested"]').attributes('aria-level')).toBe('2');
    expect(wrapper.get('[role="treeitem"][aria-label="Folder: Deep"]').attributes('aria-level')).toBe('3');
    expect(wrapper.get('[role="treeitem"][aria-label="Folder: Orphan"]').attributes('aria-level')).toBe('1');
  });

  it('aligns root inline creation with root display rows', async () => {
    const wrapper = mount(NotesTree, {
      props: { projectName: 'Atlas', folders: [], docs: [], boards: [], activeSlug: null },
    });

    await wrapper.get('button[aria-label="New page or folder"]').trigger('click');
    const createPage = [...document.body.querySelectorAll<HTMLElement>('[role="menuitem"]')].find((item) =>
      item.textContent?.includes('New page'),
    );
    await createPage?.click();

    expect((wrapper.get('.notes-inline-edit').element as HTMLElement).style.paddingLeft).toBe('28px');
    expect(wrapper.get('.notes-inline-spacer').attributes('style')).toContain('width: 12px');
    wrapper.unmount();
  });

  it('writes a board drag payload so a board can be dropped into a folder', async () => {
    const wrapper = mount(NotesTree, {
      props: {
        projectName: 'Atlas',
        folders: [],
        docs: [],
        boards: [{ id: 'b1', name: 'Roadmap', folder_id: null, task_count: 0 }],
        activeSlug: null,
      },
    });

    const boardDnd = wrapper.findAll('.tree-dnd').find((el) => el.text().includes('Roadmap'));
    let stored = '';
    await boardDnd?.trigger('dragstart', {
      dataTransfer: {
        setData: (_type: string, val: string) => {
          stored = val;
        },
        effectAllowed: '',
      },
    });

    expect(JSON.parse(stored)).toEqual({ nodes: [{ type: 'board', id: 'b1' }] });
  });

  it('ctrl-click selects a doc without opening it', async () => {
    const wrapper = mount(NotesTree, {
      props: {
        projectName: 'Atlas',
        folders: [],
        docs: [{ id: 'd2', title: 'Root note', slug: 'root-note', folder_id: null }],
        activeSlug: null,
      },
    });

    const selection = useTreeSelection();
    const docButton = wrapper.findAll('button').find((b) => b.text().includes('Root note'));
    await docButton?.trigger('click', { ctrlKey: true });

    expect(wrapper.emitted('select-doc')).toBeUndefined();
    expect(selection.isSelected(docKey('root-note'))).toBe(true);
  });
});
