import { EditorSelection } from '@codemirror/state';
import { describe, expect, it } from 'vitest';
import { restoreSelection, snapshotSelection } from '@/lib/editorSelection';

describe('editor selection snapshots', () => {
  it('restores the current cursor position after an external document refresh', () => {
    const snapshot = snapshotSelection(EditorSelection.create([EditorSelection.cursor(7)]));

    const restored = restoreSelection(snapshot, 20);

    expect(restored.main.anchor).toBe(7);
    expect(restored.main.head).toBe(7);
  });

  it('clamps the cursor when refreshed content is shorter', () => {
    const snapshot = snapshotSelection(EditorSelection.create([EditorSelection.cursor(12)]));

    const restored = restoreSelection(snapshot, 5);

    expect(restored.main.anchor).toBe(5);
    expect(restored.main.head).toBe(5);
  });

  it('preserves non-empty selections', () => {
    const snapshot = snapshotSelection(EditorSelection.create([EditorSelection.range(2, 8)]));

    const restored = restoreSelection(snapshot, 20);

    expect(restored.main.anchor).toBe(2);
    expect(restored.main.head).toBe(8);
  });
});
