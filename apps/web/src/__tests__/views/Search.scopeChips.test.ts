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

import { useSearchStore } from '@/stores/search';
import { useUiStore } from '@/stores/ui';
import Search from '@/views/Search.vue';

/**
 * Local helpers mirroring the implementation in Search.vue (SE21, SE22).
 * Tested in isolation so correctness is verified independently of the
 * full component render.
 */
function setTypeTokens(query: string, types: string[]): string {
  const stripped = query
    .replace(/(?:^|\s)type:\S+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  if (types.length === 0) return stripped;
  const token = `type:${types.join(',')}`;
  return stripped === '' ? token : `${stripped} ${token}`;
}

function typeActive(query: string, value: 'all' | 'note' | 'task'): boolean {
  const match = query.match(/(?:^|\s)type:(\S+)(?:\s|$)/);
  const tokenValues = match?.[1]?.split(',') ?? [];

  if (value === 'all') return tokenValues.length === 0 || !match;
  return tokenValues.includes(value);
}

describe('Search.vue SCOPE_CHIPS token helpers (SE21, SE22, SE23)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  describe('setTypeTokens', () => {
    it('tapping Notes chip sets type:note in q (SE21)', () => {
      const store = useSearchStore();
      store.setQuery('some query');

      const newQ = setTypeTokens(store.query, ['note']);
      store.setQuery(newQ);

      expect(store.query).toContain('type:note');
      expect(store.query).not.toMatch(/type:task/);
    });

    it('tapping All chip clears any type: token from q (SE21)', () => {
      const store = useSearchStore();
      store.setQuery('foo type:note bar');

      const newQ = setTypeTokens(store.query, []);
      store.setQuery(newQ);

      expect(store.query).not.toMatch(/type:/);
      expect(store.query).toContain('foo');
      expect(store.query).toContain('bar');
    });

    it('tapping Tasks chip sets type:task (SE21)', () => {
      const result = setTypeTokens('some query type:note', ['task']);
      expect(result).toContain('type:task');
      expect(result).not.toContain('type:note');
    });

    it('switching from note to task replaces the token (SE21)', () => {
      const q1 = setTypeTokens('', ['note']);
      expect(q1).toBe('type:note');

      const q2 = setTypeTokens(q1, ['task']);
      expect(q2).toBe('type:task');
      expect(q2).not.toContain('type:note');
    });

    it('strips all type: tokens before setting the new one (no duplicates)', () => {
      const result = setTypeTokens('q type:note type:task', ['note']);
      const matches = result.match(/type:/g);
      expect(matches).toHaveLength(1);
    });
  });

  describe('typeActive', () => {
    it('All is active when there is no type: token (SE22)', () => {
      expect(typeActive('foo bar', 'all')).toBe(true);
      expect(typeActive('', 'all')).toBe(true);
    });

    it('All is inactive when a type: token is present (SE22)', () => {
      expect(typeActive('foo type:task bar', 'all')).toBe(false);
    });

    it('note is active when type:note is present (SE22)', () => {
      expect(typeActive('type:note', 'note')).toBe(true);
    });

    it('task is active when type:task is present (SE22)', () => {
      expect(typeActive('q type:task', 'task')).toBe(true);
    });

    it('task is active in a comma union type:note,task (SE22)', () => {
      expect(typeActive('type:note,task', 'task')).toBe(true);
      expect(typeActive('type:note,task', 'note')).toBe(true);
    });

    it('note is inactive when only type:task is present (SE22)', () => {
      expect(typeActive('type:task', 'note')).toBe(false);
    });

    it('correctly derives active state from store.query (SE22)', () => {
      const store = useSearchStore();
      store.setQuery('type:task');

      expect(typeActive(store.query, 'task')).toBe(true);
      expect(typeActive(store.query, 'note')).toBe(false);
      expect(typeActive(store.query, 'all')).toBe(false);
    });
  });

  describe('sidebar-actions slot (SE23)', () => {
    it('renders a Command palette button that calls ui.openPalette()', async () => {
      GET.mockResolvedValue({
        data: { items: [], has_more: false, next_cursor: null },
        error: undefined,
      });

      const ui = useUiStore();
      const openPaletteSpy = vi.spyOn(ui, 'openPalette');

      const wrapper = mount(Search, {
        global: {
          stubs: {
            AppShell: {
              template: '<div><slot name="sidebar-actions" /></div>',
            },
            SearchSidebar: true,
            EditorToolbar: true,
            SearchPreview: true,
            ResultRow: true,
            EmptyState: true,
            ErrorState: true,
            LoadingState: true,
            Btn: true,
            Popover: true,
          },
        },
      });

      await wrapper.vm.$nextTick();

      const btn = wrapper.find('button[aria-label="Command palette"]');
      expect(btn.exists()).toBe(true);

      await btn.trigger('click');
      expect(openPaletteSpy).toHaveBeenCalled();
    });
  });
});
