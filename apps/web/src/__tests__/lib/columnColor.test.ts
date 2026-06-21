import { describe, expect, it } from 'vitest';
import { resolveColumnSwatchId } from '@/lib/columnColor';
import { defaultSwatchId } from '@/lib/swatches';
import type { ColumnDto } from '@/stores/boards';

const column = (overrides: Partial<ColumnDto> = {}): ColumnDto => ({
  id: 'col-1',
  board_id: 'board-1',
  name: 'In progress',
  position_key: 'n',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  ...overrides,
});

describe('resolveColumnSwatchId', () => {
  it('returns the backend color when set', () => {
    expect(resolveColumnSwatchId(column({ color: 'green' }))).toBe('green');
  });

  it('falls back to the deterministic default keyed by column id when color is null', () => {
    const col = column({ id: 'abc', color: null });
    expect(resolveColumnSwatchId(col)).toBe(defaultSwatchId('status:abc'));
  });

  it('falls back to the deterministic default when color is undefined', () => {
    const col = column({ id: 'xyz' });
    expect(resolveColumnSwatchId(col)).toBe(defaultSwatchId('status:xyz'));
  });

  it('the default is stable for the same id', () => {
    expect(resolveColumnSwatchId(column({ id: 'same' }))).toBe(
      resolveColumnSwatchId(column({ id: 'same', name: 'Different name' })),
    );
  });
});
