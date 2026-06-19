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
