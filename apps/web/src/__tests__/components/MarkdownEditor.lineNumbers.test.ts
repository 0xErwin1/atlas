import { mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { describe, expect, it } from 'vitest';
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';

/**
 * The line-number gutter is opt-in per host: a note is addressed by line
 * elsewhere in Atlas (`read_document_lines`), a task description is not.
 */

function editor(lineNumbers: boolean) {
  return mount(MarkdownEditor, {
    props: { body: 'one\ntwo\nthree', embeddedControls: false, lineNumbers },
    global: { plugins: [createPinia()] },
    attachTo: document.body,
  });
}

/** The rendered numbers, minus CodeMirror's hidden width-measuring spacer. */
function gutterNumbers(wrapper: ReturnType<typeof editor>): string[] {
  return wrapper
    .findAll('.cm-lineNumbers .cm-gutterElement')
    .filter((element) => !(element.attributes('style') ?? '').includes('visibility: hidden'))
    .map((element) => element.text())
    .filter((text) => text !== '');
}

describe('MarkdownEditor line numbers', () => {
  it('is absent by default, so embedded hosts keep a bare writing surface', () => {
    const wrapper = editor(false);

    expect(wrapper.find('.cm-lineNumbers').exists()).toBe(false);

    wrapper.unmount();
  });

  it('numbers every line when the host asks for the gutter', () => {
    const wrapper = editor(true);

    expect(gutterNumbers(wrapper)).toEqual(['1', '2', '3']);

    wrapper.unmount();
  });

  it('appears and disappears without rebuilding the editor', async () => {
    const wrapper = editor(false);

    await wrapper.setProps({ lineNumbers: true });
    expect(wrapper.find('.cm-lineNumbers').exists()).toBe(true);

    await wrapper.setProps({ lineNumbers: false });
    expect(wrapper.find('.cm-lineNumbers').exists()).toBe(false);

    wrapper.unmount();
  });
});
