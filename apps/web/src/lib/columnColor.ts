import { defaultSwatchId } from '@/lib/swatches';
import type { ColumnDto } from '@/stores/boards';

/**
 * Resolves the swatch id used to color a board column (status). The backend
 * `color` field is the single source of truth; when it is null/undefined the
 * column falls back to the deterministic, content-independent default keyed by
 * the column id — the same default the rest of the UI derives, so a column with
 * no explicit color looks identical everywhere instead of regressing to gray.
 */
export function resolveColumnSwatchId(column: ColumnDto): string {
  return column.color ?? defaultSwatchId(`status:${column.id}`);
}
