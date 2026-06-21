import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({
  GET: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

vi.mock('vue-router', () => ({
  useRoute: () => ({ params: {} }),
  useRouter: () => ({ push: vi.fn() }),
}));

vi.mock('@/composables/useBreakpoint', () => ({
  useBreakpoint: () => ({ isMobile: false }),
}));

import { useSavedSearchesStore } from '@/stores/savedSearches';
import { useSearchStore } from '@/stores/search';
import { useWorkspaceStore } from '@/stores/workspace';
import Search from '@/views/Search.vue';

const savedSearchDto = {
  id: 's1',
  name: 'Open notes',
  query: 'urgent type:note',
  workspace_id: 'ws-1',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
};

function mountSearch() {
  return mount(Search, {
    global: {
      stubs: {
        AppShell: {
          template: '<div><slot name="sidebar-footer" /></div>',
        },
        SearchSidebar: true,
        EditorToolbar: true,
        SearchPreview: true,
        ResultRow: true,
        EmptyState: true,
        ErrorState: true,
        LoadingState: true,
      },
    },
  });
}

async function openSavePopover(wrapper: ReturnType<typeof mountSearch>): Promise<void> {
  const trigger = wrapper.find('button[aria-label="Save this search"]');
  expect(trigger.exists()).toBe(true);
  await trigger.trigger('click');
}

describe('Search.vue Save popover (SE10–SE15)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    GET.mockResolvedValue({
      data: { items: [], has_more: false, next_cursor: null },
      error: undefined,
    });
    const ws = useWorkspaceStore();
    ws.setActiveWorkspace('ws-1');
    useSearchStore().setQuery('urgent');
  });

  it('opens an inline Popover with a name field instead of a banner (SE10)', async () => {
    const ui = (await import('@/stores/ui')).useUiStore();
    const bannerSpy = vi.spyOn(ui, 'showBanner');

    const wrapper = mountSearch();
    await wrapper.vm.$nextTick();

    await openSavePopover(wrapper);

    expect(wrapper.find('input').exists()).toBe(true);
    expect(bannerSpy).not.toHaveBeenCalled();
  });

  it('blocks submit and shows a validation error on an empty name (SE12)', async () => {
    const saved = useSavedSearchesStore();
    const createSpy = vi.spyOn(saved, 'create');

    const wrapper = mountSearch();
    await wrapper.vm.$nextTick();
    await openSavePopover(wrapper);

    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save');
    expect(saveBtn).toBeTruthy();
    await saveBtn?.trigger('click');

    expect(createSpy).not.toHaveBeenCalled();
    expect(wrapper.text()).toMatch(/required/i);
  });

  it('captures the full self-contained query on save (SE11)', async () => {
    const search = useSearchStore();
    search.setQuery('urgent status:open type:note');

    const saved = useSavedSearchesStore();
    const createSpy = vi.spyOn(saved, 'create').mockResolvedValue(savedSearchDto);

    const wrapper = mountSearch();
    await wrapper.vm.$nextTick();
    await openSavePopover(wrapper);

    await wrapper.find('input').setValue('My search');
    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save');
    await saveBtn?.trigger('click');
    await wrapper.vm.$nextTick();

    expect(createSpy).toHaveBeenCalledWith('ws-1', {
      name: 'My search',
      query: 'urgent status:open type:note',
    });
  });

  it('surfaces the backend hint inline and keeps the popover open on 409/422 (SE13, SE14)', async () => {
    const saved = useSavedSearchesStore();
    vi.spyOn(saved, 'create').mockImplementation(async () => {
      saved.error = 'A saved search with this name already exists';
      return null;
    });

    const wrapper = mountSearch();
    await wrapper.vm.$nextTick();
    await openSavePopover(wrapper);

    await wrapper.find('input').setValue('Dup name');
    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save');
    await saveBtn?.trigger('click');
    await wrapper.vm.$nextTick();

    expect(wrapper.text()).toContain('A saved search with this name already exists');
    expect(wrapper.find('input').exists()).toBe(true);
  });

  it('closes the popover on a successful save (SE15)', async () => {
    const saved = useSavedSearchesStore();
    vi.spyOn(saved, 'create').mockResolvedValue(savedSearchDto);

    const wrapper = mountSearch();
    await wrapper.vm.$nextTick();
    await openSavePopover(wrapper);

    await wrapper.find('input').setValue('Fresh name');
    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save');
    await saveBtn?.trigger('click');
    await wrapper.vm.$nextTick();
    await wrapper.vm.$nextTick();

    expect(wrapper.find('input').exists()).toBe(false);
  });
});
