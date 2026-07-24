import { mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';

const { GET, PUT } = vi.hoisted(() => ({ GET: vi.fn(), PUT: vi.fn() }));

vi.mock('@/api/wrapper', () => ({ wrappedClient: { GET, PUT } }));

vi.mock('vue-router', () => ({ useRouter: () => ({ push: vi.fn() }) }));

vi.mock('@/platform/transport', () => ({
  getPlatformTransport: () => ({ isDesktop: false }),
}));

import AccountPanel from '@/components/settings/AccountPanel.vue';
import { useUiStateStore } from '@/stores/uiState';

// The board-layout control is a Dropdown whose listbox teleports to <body>.
async function pickBoardLayout(wrapper: VueWrapper, label: string): Promise<void> {
  await wrapper.find('[data-board-layout] button').trigger('click');
  await nextTick();

  const option = Array.from(document.body.querySelectorAll<HTMLElement>('li[role="option"]')).find(
    (li) => li.textContent?.trim() === label,
  );
  if (option === undefined) throw new Error(`option not found: ${label}`);

  option.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await nextTick();
}

let activeWrapper: VueWrapper | null = null;

function mountPanel(): VueWrapper {
  const wrapper = mount(AccountPanel, { attachTo: document.body });
  activeWrapper = wrapper;
  return wrapper;
}

describe('AccountPanel — board layout preference', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    activeWrapper?.unmount();
    activeWrapper = null;
    document.body.innerHTML = '';
    vi.useRealTimers();
  });

  it('shows "Last used" when no layout is pinned', () => {
    const wrapper = mountPanel();

    expect(wrapper.find('[data-board-layout] button').text()).toContain('Last used');
  });

  it('reflects the layout already pinned in the stored ui state', async () => {
    const uiState = useUiStateStore();
    uiState.setDefaultBoardView('timeline');

    const wrapper = mountPanel();
    await nextTick();

    expect(wrapper.find('[data-board-layout] button').text()).toContain('Timeline');
  });

  it('pins the chosen layout so every board opens in it', async () => {
    const uiState = useUiStateStore();
    const wrapper = mountPanel();

    await pickBoardLayout(wrapper, 'List');

    expect(uiState.defaultBoardView()).toBe('list');
    expect(wrapper.find('[data-board-layout] button').text()).toContain('List');
  });

  it('clears the preference back to the per-board layout', async () => {
    const uiState = useUiStateStore();
    uiState.setDefaultBoardView('table');

    const wrapper = mountPanel();
    await pickBoardLayout(wrapper, 'Last used');

    expect(uiState.defaultBoardView()).toBeNull();
  });
});
