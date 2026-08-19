import { describe, expect, it } from 'vitest';
import { createStatusOptionsIndex, priorityPickerOptions } from '@/composables/taskPickerOptions';
import type { ColumnDto } from '@/stores/boards';

const column = (id: string, name: string): ColumnDto => ({
  id,
  board_id: 'board-1',
  name,
  position_key: id,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const COLUMNS = [column('c1', 'Backlog'), column('c2', 'Done')];

describe('priorityPickerOptions', () => {
  it('lists every priority plus the clear entry, marking the active one', () => {
    const options = priorityPickerOptions('high');

    expect(options.map((o) => o.value)).toEqual(['urgent', 'high', 'medium', 'low', '']);
    expect(options.filter((o) => o.active).map((o) => o.value)).toEqual(['high']);
    expect(options.at(-1)).toMatchObject({ label: 'Clear', muted: true });
  });

  it('marks nothing active for a cleared or unknown priority', () => {
    for (const priority of [null, undefined, 'bogus']) {
      expect(priorityPickerOptions(priority).some((o) => o.active)).toBe(false);
    }
  });

  it('returns the same array for the same priority so rows keep their props identity', () => {
    expect(priorityPickerOptions('low')).toBe(priorityPickerOptions('low'));
    expect(priorityPickerOptions(null)).toBe(priorityPickerOptions(undefined));
    expect(priorityPickerOptions('low')).not.toBe(priorityPickerOptions('high'));
  });
});

describe('createStatusOptionsIndex', () => {
  it('maps every column to an option, marking the active column', () => {
    const options = createStatusOptionsIndex(COLUMNS)('c2');

    expect(options.map((o) => o.value)).toEqual(['c1', 'c2']);
    expect(options.map((o) => o.label)).toEqual(['Backlog', 'Done']);
    expect(options.filter((o) => o.active).map((o) => o.value)).toEqual(['c2']);
    expect(options[0]?.color).toBeTypeOf('string');
  });

  it('marks nothing active for a column outside the board', () => {
    expect(createStatusOptionsIndex(COLUMNS)('elsewhere').some((o) => o.active)).toBe(false);
  });

  it('returns an empty list for a board with no columns', () => {
    expect(createStatusOptionsIndex([])('c1')).toEqual([]);
  });

  it('returns the same array for repeated lookups of the same active column', () => {
    const index = createStatusOptionsIndex(COLUMNS);

    expect(index('c1')).toBe(index('c1'));
    expect(index('nope')).toBe(index('other-missing'));
    expect(index('c1')).not.toBe(index('c2'));
  });
});
