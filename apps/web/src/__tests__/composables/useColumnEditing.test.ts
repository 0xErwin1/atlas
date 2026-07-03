import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET: vi.fn(), POST: vi.fn(), PATCH: vi.fn(), DELETE: vi.fn() },
}));

import { useColumnEditing } from '@/composables/useColumnEditing';
import { resolveColumnSwatchId } from '@/lib/columnColor';
import { type ColumnDto, useBoardsStore } from '@/stores/boards';

const col = (id: string, name: string, pos: string, color: string | null = null): ColumnDto => ({
  id,
  board_id: 'b1',
  name,
  position_key: pos,
  color,
  created_at: 'x',
  updated_at: 'x',
});

function setup(columns: ColumnDto[]) {
  const boards = useBoardsStore();
  boards.columns = columns;
  const edit = useColumnEditing(
    () => 'ws',
    () => 'b1',
  );
  return { boards, edit };
}

describe('useColumnEditing', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  describe('saveEdit', () => {
    it('sends only the changed name, trimmed', async () => {
      const { boards, edit } = setup([col('c1', 'Todo', 'a', 'green')]);
      const update = vi.spyOn(boards, 'updateColumn').mockResolvedValue(true);

      const column = col('c1', 'Todo', 'a', 'green');
      const ok = await edit.saveEdit(column, {
        name: '  Backlog  ',
        color: resolveColumnSwatchId(column),
      });

      expect(ok).toBe(true);
      expect(update).toHaveBeenCalledWith('ws', 'b1', 'c1', { name: 'Backlog' });
    });

    it('sends only the changed color', async () => {
      const { boards, edit } = setup([col('c1', 'Todo', 'a', 'green')]);
      const update = vi.spyOn(boards, 'updateColumn').mockResolvedValue(true);

      const ok = await edit.saveEdit(col('c1', 'Todo', 'a', 'green'), {
        name: 'Todo',
        color: 'blue',
      });

      expect(ok).toBe(true);
      expect(update).toHaveBeenCalledWith('ws', 'b1', 'c1', { color: 'blue' });
    });

    it('is a no-op when nothing changed', async () => {
      const { boards, edit } = setup([col('c1', 'Todo', 'a', 'green')]);
      const update = vi.spyOn(boards, 'updateColumn').mockResolvedValue(true);

      const column = col('c1', 'Todo', 'a', 'green');
      const ok = await edit.saveEdit(column, {
        name: 'Todo',
        color: resolveColumnSwatchId(column),
      });

      expect(ok).toBe(true);
      expect(update).not.toHaveBeenCalled();
    });
  });

  describe('move', () => {
    it('reorders to the middle using both neighbour keys', async () => {
      const { boards, edit } = setup([col('c1', 'A', 'a'), col('c2', 'B', 'm'), col('c3', 'C', 'z')]);
      const moveColumn = vi.spyOn(boards, 'moveColumn').mockResolvedValue(true);

      await edit.move(col('c2', 'B', 'm'), -1);
      expect(moveColumn).toHaveBeenCalledWith('ws', 'b1', 'c2', { before: null, after: 'a' });

      await edit.move(col('c2', 'B', 'm'), 1);
      expect(moveColumn).toHaveBeenCalledWith('ws', 'b1', 'c2', { before: 'z', after: null });
    });

    it('is a no-op at the list edges', async () => {
      const { boards, edit } = setup([col('c1', 'A', 'a'), col('c2', 'B', 'z')]);
      const moveColumn = vi.spyOn(boards, 'moveColumn').mockResolvedValue(true);

      expect(await edit.move(col('c1', 'A', 'a'), -1)).toBe(false);
      expect(await edit.move(col('c2', 'B', 'z'), 1)).toBe(false);
      expect(moveColumn).not.toHaveBeenCalled();
    });
  });

  it('remove deletes the column by id', async () => {
    const { boards, edit } = setup([col('c1', 'A', 'a')]);
    const del = vi.spyOn(boards, 'deleteColumn').mockResolvedValue(true);

    const ok = await edit.remove('c1');

    expect(ok).toBe(true);
    expect(del).toHaveBeenCalledWith('ws', 'b1', 'c1');
  });
});
