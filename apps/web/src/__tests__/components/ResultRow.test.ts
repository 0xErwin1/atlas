import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import ResultRow from '@/components/search/ResultRow.vue';
import type { SearchHitDto } from '@/stores/search';

const docHit = (extra: Partial<SearchHitDto> = {}): SearchHitDto => ({
  id: 'd1',
  kind: 'document',
  title: 'PRD — Atlas',
  score: 1,
  updated_at: '2026-01-01T00:00:00Z',
  ...extra,
});

const taskHit = (extra: Partial<SearchHitDto> = {}): SearchHitDto => ({
  id: 't1',
  kind: 'task',
  title: 'App rail shell',
  readable_id: 'ATL-42',
  score: 1,
  updated_at: '2026-01-01T00:00:00Z',
  ...extra,
});

describe('ResultRow (REQ-W25)', () => {
  it('renders a NOTE badge for documents and a TASK badge for tasks', () => {
    expect(mount(ResultRow, { props: { hit: docHit() } }).text()).toContain('NOTE');
    expect(mount(ResultRow, { props: { hit: taskHit() } }).text()).toContain('TASK');
  });

  it('shows the task readable_id in mono', () => {
    const wrapper = mount(ResultRow, { props: { hit: taskHit() } });
    const idSpan = wrapper.findAll('span').find((s) => s.text() === 'ATL-42');
    expect(idSpan).toBeDefined();
    expect(idSpan?.attributes('style')).toContain('var(--font-mono)');
  });

  it('renders an allowed <mark> highlight from the snippet', () => {
    const wrapper = mount(ResultRow, {
      props: { hit: docHit({ snippet: 'the <mark>app rail</mark> hosts apps' }) },
    });

    const snippet = wrapper.get('[data-testid="snippet"]');
    expect(snippet.element.querySelector('mark')?.textContent).toBe('app rail');
  });

  it('strips a malicious snippet so no XSS payload reaches the DOM', () => {
    const malicious = 'safe <img src=x onerror="alert(1)"> <script>alert(2)</script> <mark>hit</mark>';
    const wrapper = mount(ResultRow, { props: { hit: docHit({ snippet: malicious }) } });

    const snippet = wrapper.get('[data-testid="snippet"]');
    expect(snippet.element.querySelector('img')).toBeNull();
    expect(snippet.element.querySelector('script')).toBeNull();
    expect(snippet.html()).not.toContain('onerror');
    expect(snippet.element.querySelector('mark')?.textContent).toBe('hit');
  });

  it('marks the active row with the selection affordance', () => {
    const wrapper = mount(ResultRow, { props: { hit: docHit(), active: true } });
    const btn = wrapper.get('[data-kind="search-result"]');
    expect(btn.attributes('data-active')).toBe('true');
    expect(btn.attributes('style')).toContain('var(--c-selection)');
  });
});
