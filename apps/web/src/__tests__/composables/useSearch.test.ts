import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({
  GET: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import { type LocalAction, filterLocalActions, useSearch } from '@/composables/useSearch';

const page = (items: { id: string; kind: 'document' | 'task'; title: string }[]) => ({
  data: {
    items: items.map((i) => ({ ...i, score: 1, updated_at: '2026-01-01T00:00:00Z' })),
    next_cursor: null,
    has_more: false,
  },
  error: undefined,
});

describe('useSearch debounce (REQ-W23)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('debounces rapid query changes into a single API call', async () => {
    GET.mockResolvedValue(page([{ id: 'd1', kind: 'document', title: 'Shell' }]));

    const { onQueryInput } = useSearch('acme', 150);

    onQueryInput('a');
    onQueryInput('ap');
    onQueryInput('app');

    expect(GET).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(150);

    expect(GET).toHaveBeenCalledTimes(1);
    const call = GET.mock.calls[0]?.[1] as { params: { query: { q: string } } };
    expect(call.params.query.q).toBe('app');
  });

  it('runs a fresh search per settled query (cursor reset between queries)', async () => {
    GET.mockResolvedValue(page([{ id: 'd1', kind: 'document', title: 'Shell' }]));

    const { onQueryInput, store } = useSearch('acme', 100);

    onQueryInput('first');
    await vi.advanceTimersByTimeAsync(100);

    onQueryInput('second');
    await vi.advanceTimersByTimeAsync(100);

    expect(GET).toHaveBeenCalledTimes(2);
    const second = GET.mock.calls[1]?.[1] as { params: { query: { q: string; cursor?: string } } };
    expect(second.params.query.q).toBe('second');
    expect(second.params.query.cursor).toBeUndefined();
    expect(store.results).toHaveLength(1);
  });
});

describe('filterLocalActions (Q6 fuse.js local nav/actions)', () => {
  const actions: LocalAction[] = [
    { id: 'goto-notes', label: 'Go to Notes', kind: 'navigate' },
    { id: 'goto-tasks', label: 'Go to Tasks', kind: 'navigate' },
    { id: 'new-doc', label: 'New document', kind: 'action' },
    { id: 'new-task', label: 'New task', kind: 'action' },
  ];

  it('returns all actions for an empty query', () => {
    expect(filterLocalActions(actions, '')).toHaveLength(4);
  });

  it('fuzzy-matches local actions by label', () => {
    const result = filterLocalActions(actions, 'notes');
    expect(result[0]?.id).toBe('goto-notes');
  });

  it('matches partial / fuzzy input', () => {
    const result = filterLocalActions(actions, 'new doc');
    expect(result.some((a) => a.id === 'new-doc')).toBe(true);
  });

  it('returns nothing for a non-matching query', () => {
    expect(filterLocalActions(actions, 'zzzqqq')).toHaveLength(0);
  });
});
