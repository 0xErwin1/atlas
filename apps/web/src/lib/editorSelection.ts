import { EditorSelection, type SelectionRange } from '@codemirror/state';

export interface SelectionSnapshotRange {
  anchor: number;
  head: number;
}

export interface SelectionSnapshot {
  ranges: SelectionSnapshotRange[];
  mainIndex: number;
}

function clampOffset(offset: number, docLength: number): number {
  return Math.min(Math.max(offset, 0), docLength);
}

export function snapshotSelection(selection: EditorSelection): SelectionSnapshot {
  return {
    ranges: selection.ranges.map((range) => ({ anchor: range.anchor, head: range.head })),
    mainIndex: selection.mainIndex,
  };
}

export function restoreSelection(snapshot: SelectionSnapshot, docLength: number): EditorSelection {
  const ranges: SelectionRange[] = snapshot.ranges.map((range) =>
    EditorSelection.range(clampOffset(range.anchor, docLength), clampOffset(range.head, docLength)),
  );

  return EditorSelection.create(ranges.length > 0 ? ranges : [EditorSelection.cursor(0)], snapshot.mainIndex);
}
