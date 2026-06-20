/**
 * Label/status/tag color palette. Colors are a USER CHOICE, never inferred from
 * the value's text. Each swatch maps to the DBFlux semantic tokens so chips stay
 * on-theme in both light and dark. The persisted choice lives in the labelColors
 * store; this module is the pure palette + the deterministic default.
 */

export interface Swatch {
  id: string;
  label: string;
  fg: string;
  bg: string;
  border: string;
}

const NEUTRAL_SWATCH: Swatch = {
  id: 'neutral',
  label: 'Gray',
  fg: 'var(--c-foreground)',
  bg: 'rgba(179, 177, 173, 0.06)',
  border: 'var(--c-border)',
};

export const SWATCHES: Swatch[] = [
  NEUTRAL_SWATCH,
  {
    id: 'blue',
    label: 'Blue',
    fg: 'var(--c-info)',
    bg: 'rgba(89, 194, 255, 0.12)',
    border: 'rgba(89, 194, 255, 0.4)',
  },
  {
    id: 'green',
    label: 'Green',
    fg: 'var(--c-success)',
    bg: 'rgba(170, 217, 76, 0.12)',
    border: 'rgba(170, 217, 76, 0.4)',
  },
  {
    id: 'amber',
    label: 'Amber',
    fg: 'var(--c-primary)',
    bg: 'rgba(255, 180, 84, 0.12)',
    border: 'rgba(255, 180, 84, 0.4)',
  },
  {
    id: 'red',
    label: 'Red',
    fg: 'var(--c-danger)',
    bg: 'rgba(240, 113, 120, 0.12)',
    border: 'rgba(240, 113, 120, 0.4)',
  },
  {
    id: 'magenta',
    label: 'Magenta',
    fg: 'var(--c-agent)',
    bg: 'var(--c-agent-bg)',
    border: 'var(--c-agent-border)',
  },
  {
    id: 'cyan',
    label: 'Cyan',
    fg: 'var(--c-cyan)',
    bg: 'rgba(149, 230, 203, 0.12)',
    border: 'rgba(149, 230, 203, 0.4)',
  },
];

const SWATCH_BY_ID = new Map(SWATCHES.map((s) => [s.id, s]));

// Colored swatches only (excludes neutral) for the deterministic default, so a
// fresh tag gets a stable color instead of all-gray — still fully overridable.
const DEFAULT_POOL = SWATCHES.filter((s) => s.id !== 'neutral');

export function swatchById(id: string | undefined): Swatch {
  return (id !== undefined ? SWATCH_BY_ID.get(id) : undefined) ?? NEUTRAL_SWATCH;
}

/**
 * Stable, content-derived default color for a key with no explicit choice. This
 * is a hash over the key — NOT a semantic reading of the text — so "done" is not
 * forced to green; it is simply consistent until the user picks a color.
 */
export function defaultSwatchId(key: string): string {
  let hash = 0;
  for (let i = 0; i < key.length; i += 1) {
    hash = (hash * 31 + key.charCodeAt(i)) | 0;
  }
  const index = Math.abs(hash) % DEFAULT_POOL.length;
  return (DEFAULT_POOL[index] ?? NEUTRAL_SWATCH).id;
}
