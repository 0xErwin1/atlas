import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const css = readFileSync(resolve(process.cwd(), 'src/theme/tokens.css'), 'utf8');

/**
 * Reads the custom properties declared inside one top-level rule of
 * `tokens.css`. The file is authored as two flat blocks (`:root` for dark,
 * `:root[data-theme='light']` for light), so a brace-balanced scan from the
 * selector is enough and keeps the test free of a CSS parser dependency.
 */
function declarations(selector: string): Record<string, string> {
  const start = css.indexOf(`${selector} {`);
  expect(start, `missing rule: ${selector}`).toBeGreaterThan(-1);

  const open = css.indexOf('{', start);
  let depth = 0;
  let end = open;
  for (let i = open; i < css.length; i += 1) {
    if (css[i] === '{') depth += 1;
    if (css[i] === '}') {
      depth -= 1;
      if (depth === 0) {
        end = i;
        break;
      }
    }
  }

  const out: Record<string, string> = {};
  for (const match of css.slice(open, end).matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    const [, name, value] = match;
    if (name && value) out[name] = value.trim();
  }
  return out;
}

const dark = declarations(':root');
const light = declarations(":root[data-theme='light']");

function token(tokens: Record<string, string>, name: string): string {
  const value = tokens[name];
  if (value === undefined) throw new Error(`missing token: ${name}`);
  return value;
}

function relativeLuminance(hex: string): number {
  const [r, g, b] = [1, 3, 5]
    .map((i) => Number.parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
  const [la, lb] = [relativeLuminance(a), relativeLuminance(b)];
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

describe('Blueprint palette', () => {
  const expected: Record<string, [string, string]> = {
    '--c-background': ['#0B1015', '#FFFFFF'],
    '--c-panel': ['#0E151C', '#F4F6F7'],
    '--c-raised': ['#131C25', '#EAEEF0'],
    '--c-tiles': ['#10171F', '#F9FAFB'],
    '--c-foreground': ['#C3CDD6', '#141A20'],
    '--c-muted': ['#7A8B9B', '#4A5560'],
    '--c-border': ['#22303D', '#A8B2BC'],
    '--c-selection': ['#16222E', '#DCE6E8'],
    '--c-primary': ['#56B6C2', '#0F5F6B'],
    '--c-primary-fg': ['#0B1015', '#FFFFFF'],
    '--c-danger': ['#E06C75', '#9B2C2C'],
    '--c-success': ['#8FBF6B', '#3F6B24'],
    '--c-warning': ['#D6A15A', '#8A5A16'],
    '--c-info': ['#6FA8DC', '#1F5C8B'],
    '--c-agent': ['#A48BC8', '#5B4A80'],
  };

  it.each(Object.entries(expected))('%s', (name, [inDark, inLight]) => {
    expect(dark[name]).toBe(inDark);
    expect(light[name]).toBe(inLight);
  });

  it('keeps every semantic colour readable on its own surface', () => {
    for (const [theme, tokens] of [
      ['dark', dark],
      ['light', light],
    ] as const) {
      const background = token(tokens, '--c-background');
      const primary = token(tokens, '--c-primary');
      expect(
        contrast(token(tokens, '--c-foreground'), background),
        `${theme} foreground`,
      ).toBeGreaterThanOrEqual(7);
      expect(contrast(token(tokens, '--c-muted'), background), `${theme} muted`).toBeGreaterThanOrEqual(4.5);
      expect(contrast(primary, background), `${theme} primary`).toBeGreaterThanOrEqual(4.5);
      expect(
        contrast(token(tokens, '--c-primary-fg'), primary),
        `${theme} primary-fg`,
      ).toBeGreaterThanOrEqual(4.5);
    }
  });
});

describe('informative boundaries', () => {
  // `--c-border` draws decorative structure and stays quiet on purpose. Borders
  // that carry meaning on their own — table rules, cell edges, focus outlines —
  // use `--c-border-strong`, which has to clear the 3:1 WCAG threshold for
  // non-text contrast against every surface it can be drawn on.
  const surfaces = ['--c-background', '--c-panel', '--c-raised', '--c-selection'];

  it.each([
    ['dark', () => dark],
    ['light', () => light],
  ])('%s border-strong clears 3:1 on every surface', (_theme, read) => {
    const tokens = read();
    const strong = token(tokens, '--c-border-strong');
    for (const surface of surfaces) {
      expect(contrast(strong, token(tokens, surface)), surface).toBeGreaterThanOrEqual(3);
    }
  });
});

describe('Blueprint geometry', () => {
  it('removes every radius except the status ring', () => {
    expect(dark['--r-sm']).toBe('0');
    expect(dark['--r-md']).toBe('0');
    expect(dark['--r-lg']).toBe('0');
    expect(dark['--r-full']).toBe('9999px');
  });

  it('drops drop shadows and makes the focus ring a 2px primary outline', () => {
    expect(dark['--shadow-md']).toBe('none');
    expect(dark['--shadow-lg']).toBe('none');
    expect(dark['--shadow-ring']).toBe('0 0 0 2px var(--c-primary)');
    // The light theme overrides colours only, so it must not reintroduce depth.
    expect(light['--shadow-md']).toBeUndefined();
    expect(light['--shadow-lg']).toBeUndefined();
  });

  it('tightens the control heights', () => {
    expect(dark['--h-row']).toBe('22px');
    expect(dark['--h-compact']).toBe('20px');
    expect(dark['--h-header']).toBe('28px');
    expect(dark['--h-toolbar']).toBe('32px');
    expect(dark['--h-tab']).toBe('24px');
    expect(dark['--h-input']).toBe('26px');
    expect(dark['--h-button']).toBe('26px');
  });
});

describe('typography', () => {
  it('reads prose in IBM Plex Sans and data in IBM Plex Mono', () => {
    expect(dark['--font-ui']).toContain('IBM Plex Sans');
    expect(dark['--font-sans']).toContain('IBM Plex Sans');
    expect(dark['--font-mono']).toContain('IBM Plex Mono');
    expect(dark['--font-code']).toContain('IBM Plex Mono');
  });

  it('self-hosts every face', () => {
    const faces = [...css.matchAll(/@font-face\s*{[^}]*}/g)].map((m) => m[0]);
    expect(faces.length).toBeGreaterThan(0);
    for (const face of faces) {
      expect(face).toMatch(/src:\s*url\('\.\.\/assets\/fonts\//);
    }
    expect(css).not.toMatch(/https?:\/\//);
  });

  it('declares IBM Plex Sans as a single variable face per subset', () => {
    const sans = [...css.matchAll(/@font-face\s*{[^}]*IBM Plex Sans[^}]*}/g)].map((m) => m[0]);
    expect(sans).toHaveLength(2);
    for (const face of sans) {
      expect(face).toContain('font-weight: 100 700');
    }
  });
});
