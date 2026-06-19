import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { defineComponent, nextTick } from 'vue';
import { useInlineEdit } from '@/composables/useInlineEdit';

// The sidebar inline inputs live inside `v-for` loops, where Vue populates a
// template `ref` as an array. This harness mirrors that exact shape so the
// focus behaviour is exercised the way the real sidebars use it.
const Harness = defineComponent({
  setup() {
    const { active, value, inputRef, start, commit, onKeydown } = useInlineEdit<{ k: string }>(() => {});
    return { active, value, inputRef, start, commit, onKeydown, items: ['only'] };
  },
  template: `
    <div>
      <template v-for="x in items" :key="x">
        <input
          v-if="active"
          ref="inputRef"
          v-model="value"
          class="inline"
          @keydown="onKeydown"
        />
      </template>
    </div>
  `,
});

describe('useInlineEdit', () => {
  it('focuses the input when an edit starts inside a v-for', async () => {
    const wrapper = mount(Harness, { attachTo: document.body });

    (wrapper.vm as unknown as { start: (c: { k: string }) => void }).start({ k: 'x' });
    await nextTick();
    await nextTick();

    const input = wrapper.find('input.inline');
    expect(input.exists()).toBe(true);
    expect(document.activeElement).toBe(input.element);

    wrapper.unmount();
  });

  it('selects the existing text when starting a rename', async () => {
    const wrapper = mount(Harness, { attachTo: document.body });

    (wrapper.vm as unknown as { start: (c: { k: string }, v: string, s: boolean) => void }).start(
      { k: 'rename' },
      'Existing name',
      true,
    );
    await nextTick();
    await nextTick();

    const input = wrapper.find('input.inline').element as HTMLInputElement;
    expect(document.activeElement).toBe(input);
    expect(input.selectionStart).toBe(0);
    expect(input.selectionEnd).toBe('Existing name'.length);

    wrapper.unmount();
  });
});
