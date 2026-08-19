import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@/stores/ui', () => ({ useUiStore: () => ({ openShare: vi.fn() }) }));

import EditorToolbar from '@/components/shell/EditorToolbar.vue';

function mountToolbar(slots: Record<string, string> = {}) {
  return mount(EditorToolbar, {
    props: { breadcrumbs: ['workspace', 'note'] },
    global: { stubs: { Crumb: true, Icon: true } },
    slots,
  });
}

describe('EditorToolbar', () => {
  // The document's revalidation status rides here rather than in the reading
  // column: this row's height is fixed, so a status appearing and clearing
  // cannot push the prose down and pull it back.
  it('is a row of fixed height', () => {
    const style = mountToolbar().get('div').attributes('style') ?? '';

    expect(style).toContain('height: var(--h-toolbar)');
    expect(style).toContain('flex: 0 0 var(--h-toolbar)');
  });

  it('renders lead content next to the breadcrumb', () => {
    const wrapper = mountToolbar({ lead: '<span class="probe">Updating…</span>' });

    expect(wrapper.find('.probe').exists()).toBe(true);
  });
});
