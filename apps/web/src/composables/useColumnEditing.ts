import { resolveColumnSwatchId } from '@/lib/columnColor';
import { type ColumnDto, useBoardsStore } from '@/stores/boards';
import { useUiStore } from '@/stores/ui';

/**
 * Shared orchestration for editing a board's status columns — rename, recolor,
 * reorder and delete — against the boards store.
 *
 * Used by both the workspace Statuses settings panel and the kanban board's
 * column headers so the non-trivial parts (the changed-field diff for a save and
 * the fractional position-key math for a reorder) live in exactly one place
 * instead of being duplicated per surface.
 *
 * `ws` and `boardId` are getters so a caller can pass reactive sources (a
 * selected-board ref, the active board on the store) and always read the current
 * value at call time.
 */
export function useColumnEditing(ws: () => string | null, boardId: () => string | null) {
  const boards = useBoardsStore();
  const ui = useUiStore();

  /**
   * Persists a name and/or color edited together. Only fields that actually
   * changed are sent, so an untouched name or color is left as-is on the server.
   * Returns `true` when the column is already up to date or the update succeeds,
   * `false` on failure.
   */
  async function saveEdit(column: ColumnDto, draft: { name?: string; color?: string }): Promise<boolean> {
    const slug = ws();
    const board = boardId();
    if (slug === null || board === null) return false;

    const patch: { name?: string; color?: string } = {};

    if (draft.name !== undefined) {
      const next = draft.name.trim();
      if (next !== '' && next !== column.name) patch.name = next;
    }
    if (draft.color !== undefined && draft.color !== resolveColumnSwatchId(column)) {
      patch.color = draft.color;
    }

    if (patch.name === undefined && patch.color === undefined) return true;

    const ok = await boards.updateColumn(slug, board, column.id, patch);
    if (ok) ui.showBanner('Status updated', 'success');
    else if (boards.error) ui.showBanner(boards.error, 'error');
    return ok;
  }

  /**
   * Reorders a column one slot in `direction` (-1 left/up, +1 right/down) by
   * requesting a fractional position between its new neighbours. A no-op at the
   * list edges. `before` is the key the column will follow and `after` the key it
   * will precede (null at an edge).
   */
  async function move(column: ColumnDto, direction: -1 | 1): Promise<boolean> {
    const slug = ws();
    const board = boardId();
    if (slug === null || board === null) return false;

    const list = boards.columns;
    const index = list.findIndex((c) => c.id === column.id);
    const target = index + direction;
    if (index === -1 || target < 0 || target >= list.length) return false;

    const lower = direction === -1 ? list[target - 1] : list[target];
    const upper = direction === -1 ? list[target] : list[target + 1];

    const ok = await boards.moveColumn(slug, board, column.id, {
      before: lower?.position_key ?? null,
      after: upper?.position_key ?? null,
    });
    if (!ok && boards.error) ui.showBanner(boards.error, 'error');
    return ok;
  }

  async function remove(columnId: string): Promise<boolean> {
    const slug = ws();
    const board = boardId();
    if (slug === null || board === null) return false;

    const ok = await boards.deleteColumn(slug, board, columnId);
    if (ok) ui.showBanner('Status deleted', 'success');
    else if (boards.error) ui.showBanner(boards.error, 'error');
    return ok;
  }

  return { saveEdit, move, remove };
}
