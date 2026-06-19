import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import FolderPickerDialog from '@/components/notas/FolderPickerDialog.vue';

const folders = [
  { id: 'f1', name: 'Specs', parent_folder_id: null },
  { id: 'f2', name: 'Drafts', parent_folder_id: 'f1' },
];

function mountDialog() {
  return mount(FolderPickerDialog, {
    props: { open: true, title: 'Move to…', folders },
    global: { stubs: { teleport: true } },
  });
}

function optionButtons(wrapper: ReturnType<typeof mountDialog>) {
  return wrapper.findAll('.atl-folder-opt');
}

describe('FolderPickerDialog', () => {
  it('lists the project root and every folder', () => {
    const wrapper = mountDialog();
    const labels = optionButtons(wrapper).map((b) => b.text());

    expect(labels).toContain('Project root');
    expect(labels).toContain('Specs');
    expect(labels).toContain('Drafts');
    wrapper.unmount();
  });

  it('confirms the project root by default', async () => {
    const wrapper = mountDialog();
    const confirm = wrapper.findAll('button').find((b) => b.text().includes('Move here'));
    await confirm?.trigger('click');

    expect(wrapper.emitted('confirm')).toEqual([[null]]);
    wrapper.unmount();
  });

  it('confirms the chosen folder id', async () => {
    const wrapper = mountDialog();
    const drafts = optionButtons(wrapper).find((b) => b.text().includes('Drafts'));
    await drafts?.trigger('click');
    const confirm = wrapper.findAll('button').find((b) => b.text().includes('Move here'));
    await confirm?.trigger('click');

    expect(wrapper.emitted('confirm')).toEqual([['f2']]);
    wrapper.unmount();
  });
});
