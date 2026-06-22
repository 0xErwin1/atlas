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

const HEX_COLOR = /^#[0-9A-Fa-f]{6}$/;

interface Rgb {
  r: number;
  g: number;
  b: number;
}

function hexToRgb(hex: string): Rgb | undefined {
  if (!HEX_COLOR.test(hex)) return undefined;

  const r = Number.parseInt(hex.slice(1, 3), 16);
  const g = Number.parseInt(hex.slice(3, 5), 16);
  const b = Number.parseInt(hex.slice(5, 7), 16);
  return { r, g, b };
}

/**
 * Builds a swatch from a user-entered `#RRGGBB` hex: the hex itself is the
 * foreground (dot/text), with a translucent fill and border derived from it so
 * a custom color reads on-theme like the named swatches. Returns undefined for
 * anything that is not a well-formed 6-digit hex, letting the caller fall back.
 */
function hexSwatch(id: string): Swatch | undefined {
  const rgb = hexToRgb(id);
  if (rgb === undefined) return undefined;

  const { r, g, b } = rgb;
  return {
    id,
    label: id,
    fg: id,
    bg: `rgba(${r}, ${g}, ${b}, 0.12)`,
    border: `rgba(${r}, ${g}, ${b}, 0.4)`,
  };
}

export function swatchById(id: string | undefined): Swatch {
  if (id === undefined) return NEUTRAL_SWATCH;

  if (id.startsWith('#')) {
    return hexSwatch(id) ?? NEUTRAL_SWATCH;
  }

  return SWATCH_BY_ID.get(id) ?? NEUTRAL_SWATCH;
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
