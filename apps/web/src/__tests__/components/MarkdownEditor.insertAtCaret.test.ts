import { mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { describe, expect, it } from 'vitest';
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';
import { blockInsertion } from '@/lib/markdownInsert';

/**
 * `insertAtCaret` is the host-driven insertion path (task attachments today). It
 * places the reference on its own block so it never lands mid-sentence.
 */

function editor(body: string, editable = true) {
  return mount(MarkdownEditor, {
    props: { body, embeddedControls: false, editable },
    global: { plugins: [createPinia()] },
  });
}

type Editor = { insertAtCaret: (text: string) => void; currentMarkdown: () => string };

describe('blockInsertion', () => {
  it('breaks the line first when the caret sits mid-sentence', () => {
    expect(blockInsertion('![diagram](/api/a)', false)).toBe('\n![diagram](/api/a)\n');
  });

  it('omits the leading break when the caret already starts a line', () => {
    expect(blockInsertion('![diagram](/api/a)', true)).toBe('![diagram](/api/a)\n');
  });
});

describe('MarkdownEditor insertAtCaret', () => {
  it('inserts as its own block at the caret, keeping the existing body below', () => {
    const wrapper = editor('Notes');
    const vm = wrapper.vm as unknown as Editor;

    vm.insertAtCaret('![diagram](/api/a)');

    expect(vm.currentMarkdown()).toBe('![diagram](/api/a)\nNotes');
  });

  it('inserts without a leading break when the caret already starts a line', () => {
    const wrapper = editor('');
    const vm = wrapper.vm as unknown as Editor;

    vm.insertAtCaret('[report.pdf](/api/a)');

    expect(vm.currentMarkdown()).toBe('[report.pdf](/api/a)\n');
  });

  it('emits the change so the host persists the inserted reference', () => {
    const wrapper = editor('');

    (wrapper.vm as unknown as Editor).insertAtCaret('[report.pdf](/api/a)');

    expect(wrapper.emitted('change')?.at(-1)).toEqual(['[report.pdf](/api/a)\n']);
  });

  it('leaves a read-only document untouched', () => {
    const wrapper = editor('Notes', false);
    const vm = wrapper.vm as unknown as Editor;

    vm.insertAtCaret('[report.pdf](/api/a)');

    expect(vm.currentMarkdown()).toBe('Notes');
  });
});
