import { describe, expect, it } from 'vitest';
import {
  collectWikilinkTitleKeys,
  detectWikilinkTrigger,
  filterWikilinkCandidates,
  formatWikilink,
  parseWikilinkInner,
  wikilinkDisplay,
  wikilinkHref,
} from '@/lib/wikilink';

describe('detectWikilinkTrigger', () => {
  it('detects a trigger with the partial query after [[', () => {
    const trigger = detectWikilinkTrigger('see [[Arch', 10);

    expect(trigger).not.toBeNull();
    expect(trigger?.query).toBe('Arch');
    expect(trigger?.from).toBe(4);
  });

  it('returns an empty query right after the opening brackets', () => {
    const trigger = detectWikilinkTrigger('[[', 2);

    expect(trigger?.query).toBe('');
    expect(trigger?.from).toBe(0);
  });

  it('returns null when there is no [[', () => {
    expect(detectWikilinkTrigger('plain text', 10)).toBeNull();
  });

  it('returns null when the link is already closed', () => {
    expect(detectWikilinkTrigger('[[Done]]', 8)).toBeNull();
  });

  it('returns null when a newline interrupts the query', () => {
    expect(detectWikilinkTrigger('[[multi\nline', 12)).toBeNull();
  });
});

describe('filterWikilinkCandidates', () => {
  const candidates = [{ title: 'Architecture' }, { title: 'Roadmap' }, { title: 'API design' }];

  it('returns all candidates for an empty query', () => {
    expect(filterWikilinkCandidates(candidates, '')).toHaveLength(3);
  });

  it('filters by case-insensitive substring', () => {
    const result = filterWikilinkCandidates(candidates, 'a');
    expect(result.map((c) => c.title)).toEqual(['Architecture', 'Roadmap', 'API design']);
  });

  it('narrows to a single match', () => {
    const result = filterWikilinkCandidates(candidates, 'road');
    expect(result.map((c) => c.title)).toEqual(['Roadmap']);
  });
});

const UUID = '019ed5fa-6df7-7201-97ce-a99abae541c1';

describe('parseWikilinkInner', () => {
  it('parses a typed task link, using the readable id as the display', () => {
    expect(parseWikilinkInner('task:ATL-80')).toEqual({
      target: { kind: 'task', readableId: 'ATL-80' },
      display: 'ATL-80',
    });
  });

  it('prefers the display half of a typed link and trims around the pipe', () => {
    expect(parseWikilinkInner('  note:incident-runbook  |  the runbook  ')).toEqual({
      target: { kind: 'note', slug: 'incident-runbook' },
      display: 'the runbook',
    });
  });

  it('normalizes the kind and the readable id case', () => {
    expect(parseWikilinkInner('TASK:atl-80').target).toEqual({ kind: 'task', readableId: 'ATL-80' });
  });

  it('keeps file names verbatim', () => {
    expect(parseWikilinkInner('file:Q3 policy.pdf').target).toEqual({
      kind: 'file',
      fileName: 'Q3 policy.pdf',
    });
  });

  it('leaves a title that merely starts with a kind word alone', () => {
    expect(parseWikilinkInner('task: rewrite the parser')).toEqual({
      target: { kind: 'title' },
      display: 'task: rewrite the parser',
    });
  });

  it('parses an id-bound link into the stable id and display title', () => {
    expect(parseWikilinkInner(`  ${UUID} | Editor test `)).toEqual({
      target: { kind: 'document', id: UUID },
      display: 'Editor test',
    });
  });

  it('treats a plain title as a title-only link', () => {
    expect(parseWikilinkInner('API Design')).toEqual({ target: { kind: 'title' }, display: 'API Design' });
  });

  it('treats a non-uuid before the pipe as a legacy title', () => {
    expect(parseWikilinkInner('Foo|Bar')).toEqual({ target: { kind: 'title' }, display: 'Foo|Bar' });
  });
});

describe('formatWikilink', () => {
  it('round-trips every form it can parse', () => {
    const forms = [
      'task:ATL-80',
      'task:ATL-80|the login bug',
      'note:incident-runbook',
      'file:policy.pdf',
      `${UUID}|Editor test`,
      'Roadmap',
    ];

    for (const inner of forms) {
      expect(formatWikilink(parseWikilinkInner(inner))).toBe(`[[${inner}]]`);
    }
  });

  it('omits a display half that would only repeat the address', () => {
    expect(formatWikilink({ target: { kind: 'task', readableId: 'ATL-80' }, display: 'ATL-80' })).toBe(
      '[[task:ATL-80]]',
    );
  });
});

describe('collectWikilinkTitleKeys', () => {
  it('collects one key per resolvable target and ignores the rest', () => {
    const md = `[[task:ATL-80]] [[note:runbook|R]] [[${UUID}|One]] [[file:a.pdf]] [[Plain]] [[task:ATL-80|again]]`;

    expect(collectWikilinkTitleKeys(md).sort()).toEqual(['task:ATL-80', 'note:runbook', UUID].sort());
  });

  it('returns an empty list when nothing addresses a titled resource', () => {
    expect(collectWikilinkTitleKeys('just [[a title]] and [[file:x.pdf]] here')).toEqual([]);
  });
});

describe('wikilinkDisplay', () => {
  it('prefers the resolved live title over the text in the markdown', () => {
    const ref = parseWikilinkInner('task:ATL-80|stale snapshot');

    expect(wikilinkDisplay(ref, { 'task:ATL-80': 'Fix the login bug' })).toBe('Fix the login bug');
  });

  it('falls back to the written display when nothing resolved', () => {
    expect(wikilinkDisplay(parseWikilinkInner('file:policy.pdf'), {})).toBe('policy.pdf');
  });
});

describe('wikilinkHref', () => {
  it('routes each addressable kind to its own view', () => {
    expect(wikilinkHref(parseWikilinkInner('task:ATL-80'))).toBe('/t/task/ATL-80');
    expect(wikilinkHref(parseWikilinkInner('note:incident-runbook'))).toBe('/n/incident-runbook');
    expect(wikilinkHref(parseWikilinkInner(`${UUID}|Whatever`))).toBe(`/n/${UUID}`);
    expect(wikilinkHref(parseWikilinkInner('API Design'))).toBe('/n/api-design');
  });

  it('has no destination for an attachment, which downloads rather than opens', () => {
    expect(wikilinkHref(parseWikilinkInner('file:policy.pdf'))).toBeNull();
  });
});
