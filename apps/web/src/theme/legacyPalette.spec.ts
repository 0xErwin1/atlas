import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The Ayu palette that Blueprint replaced, as it was written by hand inside
 * component styles. Tokens covered these values everywhere they were used
 * through `var(--c-…)`; these are the copies that a theme swap cannot reach.
 */
const RETIRED = [
  /rgba\(\s*179,\s*177,\s*173/i,
  /rgba\(\s*255,\s*180,\s*84/i,
  /rgba\(\s*89,\s*194,\s*255/i,
  /rgba\(\s*170,\s*217,\s*76/i,
  /rgba\(\s*240,\s*113,\s*120/i,
  /rgba\(\s*210,\s*166,\s*255/i,
  /#(FFB454|B3B1AD|0A0E14|F07178|AAD94C|59C2FF|D2A6FF|95E6CB|5C6773|1F2430|273747|151E2B|0F1419|111823)\b/i,
];

/**
 * Directories already carried over to Blueprint. Each restyle phase adds its
 * own, so the guard grows with the migration instead of failing on work that
 * has not started.
 */
const MIGRATED = [
  'theme',
  'components/ui',
  'components/states',
  'components/shell',
  'components/tareas',
  'components/editor',
  'components/search',
  'components/settings',
  'views',
  'lib',
];

function filesUnder(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) return filesUnder(path);
    return /\.(vue|ts|css)$/.test(entry) ? [path] : [];
  });
}

/**
 * Blueprint separates a floating surface from the content behind it with a
 * border, not with light: `--shadow-md` and `--shadow-lg` both resolve to
 * `none`. A component still asking for one has an edge nobody can see.
 */
describe('floating surfaces', () => {
  it('draw their own edge instead of a shadow that resolves to none', () => {
    const offenders = filesUnder(resolve(process.cwd(), 'src')).filter((path) =>
      /box-shadow:\s*var\(--shadow-(md|lg)/.test(readFileSync(path, 'utf8')),
    );

    expect(offenders).toEqual([]);
  });
});

/**
 * The colour the shell paints before any stylesheet loads. It lives outside the
 * token system by necessity — there is nothing to read a token from yet — so it
 * is the one place the palette has to be duplicated, and the one place a stale
 * copy shows as a flash of the old theme on every cold start.
 */
describe('pre-paint background', () => {
  it.each([
    ['index.html', 'index.html'],
    ['the desktop window', '../desktop/src-tauri/tauri.conf.json'],
  ])('%s paints the Blueprint background', (_name, file) => {
    const source = readFileSync(resolve(process.cwd(), file), 'utf8');

    expect(source.toUpperCase()).toContain('#0B1015');
    for (const pattern of RETIRED) {
      expect(pattern.test(source), `${file} still carries a retired colour`).toBe(false);
    }
  });
});

describe('retired palette', () => {
  it.each(MIGRATED)('%s carries no hand-written Ayu colour', (dir) => {
    const offenders = filesUnder(resolve(process.cwd(), 'src', dir)).filter((path) => {
      const source = readFileSync(path, 'utf8');
      return RETIRED.some((pattern) => pattern.test(source));
    });

    expect(offenders).toEqual([]);
  });
});
