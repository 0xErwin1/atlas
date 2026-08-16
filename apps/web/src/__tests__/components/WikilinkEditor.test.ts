import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import { defineComponent, h } from 'vue';
import WikilinkEditor from '@/components/editor/WikilinkEditor.vue';

/**
 * The wiring every wikilink-capable editor needs lives here once. These assert
 * the three connections a host used to re-inline: query in, selection out, and
 * click-to-navigate.
 */

const push = vi.fn();

vi.mock('vue-router', () => ({
  useRouter: () => ({ push }),
}));

vi.mock('@/composables/useWikilinkTitles', () => ({
  useWikilinkTitles: () => ({ value: {} }),
}));

const insertWikilink = vi.fn();

const MarkdownEditorStub = defineComponent({
  name: 'MarkdownEditor',
  props: { body: { type: String, default: '' } },
  emits: ['change', 'navigate-wikilink', 'wikilink-query'],
  setup: (_props, { expose }) => {
    expose({ insertWikilink, focus: vi.fn(), currentMarkdown: () => 'text', insertAtCaret: vi.fn() });
    return () => h('div', { 'data-editor': true });
  },
});

const WikiLinkSuggestStub = defineComponent({
  name: 'WikiLinkSuggest',
  props: { ws: { type: String, required: true }, query: { type: String, default: null } },
  setup: (props, { expose }) => {
    expose({ open: true, moveDown: vi.fn(), moveUp: vi.fn(), confirmActive: vi.fn() });
    return () => h('div', { 'data-suggest': props.query });
  },
});

function mountEditor() {
  return mount(WikilinkEditor, {
    props: { ws: 'atlas', body: 'hello' },
    global: {
      stubs: { MarkdownEditor: MarkdownEditorStub, WikiLinkSuggest: WikiLinkSuggestStub },
    },
  });
}

describe('WikilinkEditor', () => {
  it('shows the picker only while a [[ query is open', async () => {
    const wrapper = mountEditor();
    expect(wrapper.find('[data-suggest]').exists()).toBe(false);

    await wrapper.getComponent(MarkdownEditorStub).vm.$emit('wikilink-query', 'run', { left: 1, top: 2 });
    expect(wrapper.get('[data-suggest]').attributes('data-suggest')).toBe('run');

    await wrapper.getComponent(MarkdownEditorStub).vm.$emit('wikilink-query', null, null);
    expect(wrapper.find('[data-suggest]').exists()).toBe(false);
  });

  it('inserts the chosen reference into the editor', async () => {
    const wrapper = mountEditor();
    await wrapper.getComponent(MarkdownEditorStub).vm.$emit('wikilink-query', 'run', { left: 1, top: 2 });

    const reference = { target: { kind: 'note', slug: 'runbook' }, display: 'Runbook' };
    await wrapper.getComponent(WikiLinkSuggestStub).vm.$emit('select', reference);

    expect(insertWikilink).toHaveBeenCalledWith(reference);
  });

  it('routes a clicked wikilink by its kind', async () => {
    const wrapper = mountEditor();

    await wrapper
      .getComponent(MarkdownEditorStub)
      .vm.$emit('navigate-wikilink', { target: { kind: 'task', readableId: 'ATL-80' }, display: 'x' });

    expect(push).toHaveBeenCalledWith('/t/task/ATL-80');
  });

  it('does not navigate for an attachment, which has no page', async () => {
    push.mockClear();
    const wrapper = mountEditor();

    await wrapper
      .getComponent(MarkdownEditorStub)
      .vm.$emit('navigate-wikilink', { target: { kind: 'file', fileName: 'policy.pdf' }, display: 'x' });

    expect(push).not.toHaveBeenCalled();
  });
});
