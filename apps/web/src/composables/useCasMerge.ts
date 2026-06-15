import { applyPatch, diffLines } from 'diff';

/**
 * A single unresolved region of a 3-way merge. `base` is the common ancestor
 * text for the region, `mine` the local edit, `theirs` the reconstructed remote
 * edit. The trailing newline is stripped from each so the conflict view can
 * present whole-line text without spurious blank lines.
 */
export interface ConflictHunk {
  base: string;
  mine: string;
  theirs: string;
}

export interface MergeInput {
  /** The base content the editor loaded (body at the loaded head revision). */
  base: string;
  /** The local edited content ("mine"). */
  mine: string;
  /**
   * The server's `base_to_current_patch` (a unified diff). Applied to `base`
   * to reconstruct the current remote content ("theirs").
   */
  patch: string;
}

/**
 * The merged document expressed as an ordered list of segments. Stable segments
 * are auto-merged text; conflict segments are unresolved hunks the user must
 * choose. Concatenating the segments (after choosing a side per conflict) with
 * '\n' reproduces a valid document, so the conflict view can reassemble the
 * result without re-running the merge.
 */
export type MergeSegment = { kind: 'stable'; text: string } | { kind: 'conflict'; hunk: ConflictHunk };

export type MergeResult =
  | { kind: 'clean'; merged: string }
  | {
      kind: 'conflict';
      hunks: ConflictHunk[];
      segments: MergeSegment[];
      reconstructed: string | null;
    };

/**
 * Composable implementing the CAS 3-way merge (REQ-W18, design §7b).
 *
 * On a save conflict the editor holds the BASE it loaded and MINE (local
 * edits); the server returns a unified patch from base to its CURRENT content.
 * This composable:
 *   1. Reconstructs THEIRS by applying the patch to BASE (jsdiff applyPatch).
 *      If the patch cannot be applied it degrades to a single whole-document
 *      conflict rather than crashing or losing data.
 *   2. Performs a line-level 3-way merge of base / mine / theirs.
 *   3. Returns the merged content when every changed region is disjoint or
 *      identical on both sides (the caller auto-resaves with the current
 *      revision id), or the list of conflicting hunks otherwise.
 *
 * It NEVER applies last-write-wins and NEVER silently drops either side.
 */
export function useCasMerge() {
  function merge(input: MergeInput): MergeResult {
    const { base, mine, patch } = input;

    const reconstructed = reconstructTheirs(base, patch);

    if (reconstructed === false) {
      const hunk: ConflictHunk = { base, mine, theirs: '' };
      return {
        kind: 'conflict',
        reconstructed: null,
        hunks: [hunk],
        segments: [{ kind: 'conflict', hunk }],
      };
    }

    return threeWayMerge(base, mine, reconstructed);
  }

  return { merge };
}

function reconstructTheirs(base: string, patch: string): string | false {
  try {
    return applyPatch(base, patch);
  } catch {
    return false;
  }
}

/** A run of base lines together with how each side rewrote that run. */
interface AlignedRegion {
  base: string[];
  mine: string[];
  theirs: string[];
}

/**
 * Line-level diff3. We diff base->mine and base->theirs, then walk both diffs
 * in lockstep over the shared base lines, grouping each maximal run of base
 * lines (and the surrounding side-only insertions) into an aligned region.
 *
 * A region is a CONFLICT only when both sides changed it AND they disagree.
 * Regions changed by a single side, or identically by both, are merged
 * automatically.
 */
function threeWayMerge(base: string, mine: string, theirs: string): MergeResult {
  const rawRegions = alignSides(splitLines(base), splitLines(mine), splitLines(theirs));
  const regions = coalesceConflicts(rawRegions.flatMap(refineRegion));

  const conflicts: ConflictHunk[] = [];
  const mergedLines: string[] = [];
  const segments: MergeSegment[] = [];

  const pushStable = (lines: string[]): void => {
    if (lines.length === 0) return;
    mergedLines.push(...lines);
    const last = segments[segments.length - 1];
    const text = lines.join('\n');
    if (last !== undefined && last.kind === 'stable') {
      last.text = `${last.text}\n${text}`;
    } else {
      segments.push({ kind: 'stable', text });
    }
  };

  for (const region of regions) {
    const mineChanged = !sameLines(region.base, region.mine);
    const theirsChanged = !sameLines(region.base, region.theirs);

    if (!mineChanged && !theirsChanged) {
      pushStable(region.base);
      continue;
    }

    if (mineChanged && !theirsChanged) {
      pushStable(region.mine);
      continue;
    }

    if (theirsChanged && !mineChanged) {
      pushStable(region.theirs);
      continue;
    }

    // Both sides changed the region.
    if (sameLines(region.mine, region.theirs)) {
      pushStable(region.mine);
      continue;
    }

    const hunk: ConflictHunk = {
      base: region.base.join('\n'),
      mine: region.mine.join('\n'),
      theirs: region.theirs.join('\n'),
    };
    conflicts.push(hunk);
    segments.push({ kind: 'conflict', hunk });
    // The conflicting region contributes nothing to the auto-merged text; the
    // caller resolves it through the conflict view before resaving.
  }

  if (conflicts.length === 0) {
    return { kind: 'clean', merged: mergedLines.join('\n') };
  }

  return { kind: 'conflict', hunks: conflicts, segments, reconstructed: theirs };
}

/**
 * jsdiff coalesces adjacent changed base lines into a single block, which can
 * lump a disjoint single-side edit together with a genuine conflict. When all
 * three sides of a both-changed region have the same line count we re-align them
 * 1:1 and split per line, so a theirs-only line and a conflicting line in the
 * same block become separate regions. Regions where line counts differ (true
 * insertions/deletions) are left intact.
 */
function refineRegion(region: AlignedRegion): AlignedRegion[] {
  const mineChanged = !sameLines(region.base, region.mine);
  const theirsChanged = !sameLines(region.base, region.theirs);

  const bothChanged = mineChanged && theirsChanged;
  const lineCountsMatch =
    region.base.length === region.mine.length && region.base.length === region.theirs.length;

  if (!bothChanged || !lineCountsMatch || region.base.length <= 1) {
    return [region];
  }

  const split: AlignedRegion[] = [];
  for (let i = 0; i < region.base.length; i++) {
    const baseLine = region.base[i] ?? '';
    const mineLine = region.mine[i] ?? '';
    const theirsLine = region.theirs[i] ?? '';
    split.push({ base: [baseLine], mine: [mineLine], theirs: [theirsLine] });
  }

  return split;
}

/**
 * Merges adjacent regions that are BOTH conflicts into one, so a multi-line
 * conflict (split per line by refinement) reads as a single hunk. Non-conflict
 * regions stay separate.
 */
function coalesceConflicts(regions: AlignedRegion[]): AlignedRegion[] {
  const isConflict = (r: AlignedRegion): boolean => {
    const mineChanged = !sameLines(r.base, r.mine);
    const theirsChanged = !sameLines(r.base, r.theirs);
    return mineChanged && theirsChanged && !sameLines(r.mine, r.theirs);
  };

  const merged: AlignedRegion[] = [];

  for (const region of regions) {
    const prev = merged[merged.length - 1];

    if (prev !== undefined && isConflict(prev) && isConflict(region)) {
      prev.base.push(...region.base);
      prev.mine.push(...region.mine);
      prev.theirs.push(...region.theirs);
      continue;
    }

    merged.push({ base: [...region.base], mine: [...region.mine], theirs: [...region.theirs] });
  }

  return merged;
}

/**
 * A change block on one side, expressed in BASE coordinates: base lines in
 * `[baseStart, baseEnd)` are replaced by `replacement`. Insertions have an empty
 * base range (baseStart === baseEnd); deletions have an empty replacement.
 */
interface SideChange {
  baseStart: number;
  baseEnd: number;
  replacement: string[];
}

/**
 * Converts a base->side line diff into a list of change blocks anchored in base
 * coordinates. Lines both sides keep verbatim are NOT emitted (they are the
 * stable backbone); only divergences appear.
 */
function sideChanges(sideDiff: ReturnType<typeof diffLines>): SideChange[] {
  const changes: SideChange[] = [];
  let baseIndex = 0;
  let pendingRemovedStart: number | null = null;
  let pendingReplacement: string[] = [];

  const flush = (): void => {
    if (pendingRemovedStart === null && pendingReplacement.length === 0) return;
    const start = pendingRemovedStart ?? baseIndex;
    changes.push({ baseStart: start, baseEnd: baseIndex, replacement: pendingReplacement });
    pendingRemovedStart = null;
    pendingReplacement = [];
  };

  for (const change of sideDiff) {
    const lines = splitLines(change.value);

    if (change.removed) {
      if (pendingRemovedStart === null) pendingRemovedStart = baseIndex;
      baseIndex += lines.length;
      continue;
    }

    if (change.added) {
      pendingReplacement.push(...lines);
      continue;
    }

    // Retained run: closes any pending change block, then advances over the
    // stable base lines.
    flush();
    baseIndex += lines.length;
  }

  flush();

  return changes;
}

/**
 * Classic line-level diff3 over base coordinates. We sweep base line indices
 * from 0..base.length, and at each step take the next change block from each
 * side. Stable base lines (no side change pending) pass through unchanged;
 * where either side has a change we cut a region spanning the union of the two
 * sides' affected base ranges, so a region is self-contained and conflicts stay
 * tight.
 */
function alignSides(base: string[], mine: string[], theirs: string[]): AlignedRegion[] {
  const mineChanges = sideChanges(diffLines(base.join('\n'), mine.join('\n')));
  const theirsChanges = sideChanges(diffLines(base.join('\n'), theirs.join('\n')));

  const regions: AlignedRegion[] = [];
  let pos = 0;
  let mi = 0;
  let ti = 0;

  while (pos < base.length || mi < mineChanges.length || ti < theirsChanges.length) {
    const nextMine = mineChanges[mi];
    const nextTheirs = theirsChanges[ti];

    const mineStarts = nextMine !== undefined && nextMine.baseStart <= pos;
    const theirsStarts = nextTheirs !== undefined && nextTheirs.baseStart <= pos;

    if (!mineStarts && !theirsStarts) {
      // Stable run up to the next change (or end of base): emit verbatim.
      const nextChangeAt = Math.min(nextMine?.baseStart ?? base.length, nextTheirs?.baseStart ?? base.length);
      const stable = base.slice(pos, nextChangeAt);
      if (stable.length > 0) {
        regions.push({ base: stable, mine: [...stable], theirs: [...stable] });
      }
      pos = nextChangeAt;
      continue;
    }

    // At least one side changes at `pos`. Grow the region to cover the union of
    // the overlapping change blocks from both sides.
    let regionEnd = pos;
    const mineParts: { baseStart: number; baseEnd: number; replacement: string[] }[] = [];
    const theirsParts: { baseStart: number; baseEnd: number; replacement: string[] }[] = [];

    let grew = true;
    while (grew) {
      grew = false;

      let nextMineChange = mineChanges[mi];
      while (nextMineChange !== undefined && nextMineChange.baseStart <= regionEnd) {
        mineParts.push(nextMineChange);
        regionEnd = Math.max(regionEnd, nextMineChange.baseEnd);
        mi++;
        grew = true;
        nextMineChange = mineChanges[mi];
      }

      let nextTheirsChange = theirsChanges[ti];
      while (nextTheirsChange !== undefined && nextTheirsChange.baseStart <= regionEnd) {
        theirsParts.push(nextTheirsChange);
        regionEnd = Math.max(regionEnd, nextTheirsChange.baseEnd);
        ti++;
        grew = true;
        nextTheirsChange = theirsChanges[ti];
      }
    }

    regions.push(buildRegion(base, pos, regionEnd, mineParts, theirsParts));
    pos = regionEnd;
  }

  return regions;
}

/**
 * Materializes a region over base `[regionStart, regionEnd)` by overlaying each
 * side's change blocks onto the base slice; base lines a side did not touch
 * survive on that side.
 */
function buildRegion(
  base: string[],
  regionStart: number,
  regionEnd: number,
  mineParts: SideChange[],
  theirsParts: SideChange[],
): AlignedRegion {
  return {
    base: base.slice(regionStart, regionEnd),
    mine: overlay(base, regionStart, regionEnd, mineParts),
    theirs: overlay(base, regionStart, regionEnd, theirsParts),
  };
}

function overlay(base: string[], regionStart: number, regionEnd: number, parts: SideChange[]): string[] {
  const out: string[] = [];
  let cursor = regionStart;

  for (const part of parts) {
    if (part.baseStart > cursor) {
      out.push(...base.slice(cursor, part.baseStart));
    }
    out.push(...part.replacement);
    cursor = Math.max(cursor, part.baseEnd);
  }

  if (cursor < regionEnd) {
    out.push(...base.slice(cursor, regionEnd));
  }

  return out;
}

function splitLines(text: string): string[] {
  if (text === '') return [];
  const lines = text.split('\n');
  // A trailing newline yields a final empty element; drop it so line counts
  // align with the human notion of "lines".
  if (lines.length > 0 && lines[lines.length - 1] === '') lines.pop();
  return lines;
}

function sameLines(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}
