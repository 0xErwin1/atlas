import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';

import CasConflictView from '@/components/notas/CasConflictView.vue';
import type { MergeSegment } from '@/composables/useCasMerge';

const segments: MergeSegment[] = [
  { kind: 'stable', text: 'title' },
  {
    kind: 'conflict',
    hunk: { base: 'shared', mine: 'shared — mine', theirs: 'shared — theirs' },
  },
  { kind: 'stable', text: 'footer' },
];

function mountView(props: Partial<{ open: boolean; segments: MergeSegment[] }> = {}) {
  return mount(CasConflictView, {
    props: { open: true, segments, ...props },
  });
}

describe('CasConflictView', () => {
  it('does not render when closed', () => {
    const wrapper = mountView({ open: false });
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
  });

  it('renders both sides of every conflict hunk', () => {
    const wrapper = mountView();
    expect(wrapper.text()).toContain('shared — mine');
    expect(wrapper.text()).toContain('shared — theirs');
  });

  it('does not let the user resolve before choosing a side for every conflict (no last-write-wins)', async () => {
    const wrapper = mountView();

    const resolveBtn = wrapper.get('[data-test="resolve"]');
    expect(resolveBtn.attributes('disabled')).toBeDefined();

    await resolveBtn.trigger('click');
    expect(wrapper.emitted('resolve')).toBeUndefined();
  });

  it('assembles the local content when "mine" is chosen and emits resolve', async () => {
    const wrapper = mountView();

    await wrapper.get('[data-test="pick-mine-0"]').trigger('click');
    await wrapper.get('[data-test="resolve"]').trigger('click');

    const emitted = wrapper.emitted('resolve');
    expect(emitted).toBeTruthy();
    expect(emitted?.[0]).toEqual(['title\nshared — mine\nfooter']);
  });

  it('assembles the remote content when "theirs" is chosen', async () => {
    const wrapper = mountView();

    await wrapper.get('[data-test="pick-theirs-0"]').trigger('click');
    await wrapper.get('[data-test="resolve"]').trigger('click');

    expect(wrapper.emitted('resolve')?.[0]).toEqual(['title\nshared — theirs\nfooter']);
  });

  it('preserves a manual edit of a conflict region', async () => {
    const wrapper = mountView();

    const editor = wrapper.get('[data-test="edit-0"]');
    await editor.setValue('shared — manually reconciled');
    await wrapper.get('[data-test="resolve"]').trigger('click');

    expect(wrapper.emitted('resolve')?.[0]).toEqual(['title\nshared — manually reconciled\nfooter']);
  });

  it('emits cancel without resolving', async () => {
    const wrapper = mountView();
    await wrapper.get('[data-test="cancel"]').trigger('click');
    expect(wrapper.emitted('cancel')).toBeTruthy();
    expect(wrapper.emitted('resolve')).toBeUndefined();
  });
});
