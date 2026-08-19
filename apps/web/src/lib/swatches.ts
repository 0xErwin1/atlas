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

/**
 * Builds a named swatch from one semantic token, washed into the surface at the
 * same strengths `Chip` uses for its tones, so a tag and a status chip of the
 * same colour are the same colour.
 */
function tokenSwatch(id: string, label: string, token: string): Swatch {
  return {
    id,
    label,
    fg: `var(${token})`,
    bg: `color-mix(in srgb, var(${token}) 12%, transparent)`,
    border: `color-mix(in srgb, var(${token}) 40%, transparent)`,
  };
}

const NEUTRAL_SWATCH: Swatch = {
  id: 'neutral',
  label: 'Gray',
  fg: 'var(--c-foreground)',
  bg: 'color-mix(in srgb, var(--c-foreground) 6%, transparent)',
  border: 'var(--c-border)',
};

export const SWATCHES: Swatch[] = [
  NEUTRAL_SWATCH,
  tokenSwatch('blue', 'Blue', '--c-info'),
  tokenSwatch('green', 'Green', '--c-success'),
  tokenSwatch('amber', 'Amber', '--c-warning'),
  tokenSwatch('red', 'Red', '--c-danger'),
  tokenSwatch('magenta', 'Magenta', '--c-agent'),
  tokenSwatch('cyan', 'Cyan', '--c-cyan'),
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
