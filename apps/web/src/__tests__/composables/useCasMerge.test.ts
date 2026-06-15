import { createPatch } from 'diff';
import { describe, expect, it } from 'vitest';
import { useCasMerge } from '@/composables/useCasMerge';

/**
 * Build a real unified diff (the same shape the server sends in
 * ConflictProblem.base_to_current_patch) from base -> current content.
 * Using jsdiff's own createPatch keeps the fixtures honest: the merge code
 * has to apply a genuine patch, not a hand-rolled approximation.
 */
function patchBaseToCurrent(base: string, current: string): string {
  return createPatch('doc', base, current);
}

describe('useCasMerge', () => {
  const { merge } = useCasMerge();

  it('auto-merges disjoint edits cleanly (mine and theirs touch different regions)', () => {
    const base = ['line one', 'line two', 'line three', 'line four'].join('\n');

    // theirs (server/remote): changed the FIRST line
    const theirs = ['LINE ONE EDITED', 'line two', 'line three', 'line four'].join('\n');
    // mine (local): changed the LAST line
    const mine = ['line one', 'line two', 'line three', 'LINE FOUR EDITED'].join('\n');

    const result = merge({
      base,
      mine,
      patch: patchBaseToCurrent(base, theirs),
    });

    expect(result.kind).toBe('clean');
    if (result.kind === 'clean') {
      expect(result.merged).toBe(
        ['LINE ONE EDITED', 'line two', 'line three', 'LINE FOUR EDITED'].join('\n'),
      );
    }
  });

  it('auto-merges when only one side changed (mine untouched, theirs edited)', () => {
    const base = ['alpha', 'beta', 'gamma'].join('\n');
    const theirs = ['alpha', 'BETA CHANGED', 'gamma'].join('\n');
    const mine = base; // local made no edits

    const result = merge({ base, mine, patch: patchBaseToCurrent(base, theirs) });

    expect(result.kind).toBe('clean');
    if (result.kind === 'clean') {
      expect(result.merged).toBe(theirs);
    }
  });

  it('reports a conflict when mine and theirs change the SAME region (never silently merges)', () => {
    const base = ['title', 'shared paragraph', 'footer'].join('\n');
    const theirs = ['title', 'shared paragraph — server version', 'footer'].join('\n');
    const mine = ['title', 'shared paragraph — my version', 'footer'].join('\n');

    const result = merge({ base, mine, patch: patchBaseToCurrent(base, theirs) });

    expect(result.kind).toBe('conflict');
    if (result.kind === 'conflict') {
      expect(result.hunks.length).toBe(1);
      const hunk = result.hunks[0];
      expect(hunk?.mine).toBe('shared paragraph — my version');
      expect(hunk?.theirs).toBe('shared paragraph — server version');
      expect(hunk?.base).toBe('shared paragraph');
      // The reconstructed remote content is exposed for the conflict view.
      expect(result.reconstructed).toBe(theirs);
    }
  });

  it('exposes ordered segments so the conflict view can reassemble the document', () => {
    const base = ['title', 'shared paragraph', 'footer'].join('\n');
    const theirs = ['title', 'shared paragraph — server version', 'footer'].join('\n');
    const mine = ['title', 'shared paragraph — my version', 'footer'].join('\n');

    const result = merge({ base, mine, patch: patchBaseToCurrent(base, theirs) });

    expect(result.kind).toBe('conflict');
    if (result.kind === 'conflict') {
      expect(result.segments.map((s) => s.kind)).toEqual(['stable', 'conflict', 'stable']);

      // Picking "mine" for the conflict and concatenating segments must rebuild
      // exactly the local document.
      const rebuilt = result.segments.map((s) => (s.kind === 'stable' ? s.text : s.hunk.mine)).join('\n');
      expect(rebuilt).toBe(mine);

      // Picking "theirs" rebuilds the remote document.
      const rebuiltTheirs = result.segments
        .map((s) => (s.kind === 'stable' ? s.text : s.hunk.theirs))
        .join('\n');
      expect(rebuiltTheirs).toBe(theirs);
    }
  });

  it('separates a clean region and a conflicting region in the same document', () => {
    const base = ['intro', 'middle', 'outro'].join('\n');
    // theirs edits intro (disjoint) AND middle (conflicting)
    const theirs = ['INTRO server', 'middle server', 'outro'].join('\n');
    // mine edits middle (conflicting) only
    const mine = ['intro', 'middle mine', 'outro'].join('\n');

    const result = merge({ base, mine, patch: patchBaseToCurrent(base, theirs) });

    expect(result.kind).toBe('conflict');
    if (result.kind === 'conflict') {
      expect(result.hunks.length).toBe(1);
      expect(result.hunks[0]?.mine).toBe('middle mine');
      expect(result.hunks[0]?.theirs).toBe('middle server');
    }
  });

  it('keeps a multi-line conflicting region as a single hunk', () => {
    const base = ['header', 'l1', 'l2', 'footer'].join('\n');
    const theirs = ['header', 'l1 server', 'l2 server', 'footer'].join('\n');
    const mine = ['header', 'l1 mine', 'l2 mine', 'footer'].join('\n');

    const result = merge({ base, mine, patch: patchBaseToCurrent(base, theirs) });

    expect(result.kind).toBe('conflict');
    if (result.kind === 'conflict') {
      expect(result.hunks.length).toBe(1);
      expect(result.hunks[0]?.mine).toBe('l1 mine\nl2 mine');
      expect(result.hunks[0]?.theirs).toBe('l1 server\nl2 server');
    }
  });

  it('treats a patch that fails to apply as a conflict (no crash, no data loss)', () => {
    const base = ['one', 'two', 'three'].join('\n');
    const mine = ['one', 'two mine', 'three'].join('\n');

    // A patch whose context lines do not match `base` at all -> applyPatch returns false.
    const bogusPatch = patchBaseToCurrent('completely different content\nthat shares nothing', 'x\ny');

    const result = merge({ base, mine, patch: bogusPatch });

    expect(result.kind).toBe('conflict');
    if (result.kind === 'conflict') {
      // Defensive fallback: the whole document is one conflict, both sides preserved.
      expect(result.reconstructed).toBeNull();
      expect(result.hunks.length).toBeGreaterThanOrEqual(1);
      expect(result.hunks.some((h) => h.mine.includes('two mine'))).toBe(true);
    }
  });

  it('auto-merges when both sides make the SAME edit (identical change is not a conflict)', () => {
    const base = ['a', 'b', 'c'].join('\n');
    const sameEdit = ['a', 'B EDITED', 'c'].join('\n');

    const result = merge({
      base,
      mine: sameEdit,
      patch: patchBaseToCurrent(base, sameEdit),
    });

    expect(result.kind).toBe('clean');
    if (result.kind === 'clean') {
      expect(result.merged).toBe(sameEdit);
    }
  });
});
