function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/**
 * Maps a horizontal scroll position to the nearest column index, for the mobile
 * board's page-dot indicator. Returns 0 when there is at most one column or the
 * track does not overflow.
 */
export function activeDotIndex(scrollLeft: number, maxScroll: number, columnCount: number): number {
  if (columnCount <= 1 || maxScroll <= 0) return 0;

  const fraction = scrollLeft / maxScroll;
  const index = Math.round(fraction * (columnCount - 1));

  return clamp(index, 0, columnCount - 1);
}

/**
 * The scrollLeft a given page dot should scroll the board to, spreading the dots
 * evenly across the scrollable range.
 */
export function dotScrollTarget(index: number, maxScroll: number, columnCount: number): number {
  if (columnCount <= 1 || maxScroll <= 0) return 0;

  const clamped = clamp(index, 0, columnCount - 1);

  return (maxScroll * clamped) / (columnCount - 1);
}
