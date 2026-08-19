import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { mount } from '@vue/test-utils';
import * as lucide from 'lucide-vue-next';
import { describe, expect, it } from 'vitest';
import Icon from '@/components/ui/Icon.vue';
import { LUCIDE_ICONS } from '@/components/ui/lucideIcons';

/**
 * `Icon` resolves a kebab-case name at runtime. A namespace import of the whole
 * lucide package therefore cannot be tree-shaken and drags every icon in the set
 * into the bundle, so the component resolves through an explicit registry
 * instead. This guards the other half of that trade: an icon name used anywhere
 * in the app must have an entry, or it renders as nothing.
 */

const SOURCE_ROOT = resolve(__dirname, '../..');
const REGISTRY = relative(SOURCE_ROOT, resolve(SOURCE_ROOT, 'components/ui/lucideIcons.ts'));
const SKIPPED = ['__tests__', REGISTRY];

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (SKIPPED.some((skipped) => relative(SOURCE_ROOT, path) === skipped)) return [];
    if (entry.isDirectory()) return sourceFiles(path);
    return /\.(vue|ts)$/.test(entry.name) ? [path] : [];
  });
}

function pascalCase(name: string): string {
  return name
    .split('-')
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join('');
}

function lucideNamesUsedIn(source: string): string[] {
  const literals = source.matchAll(/['"`]([a-z][a-z0-9]*(?:-[a-z0-9]+)*)['"`]/g);
  return [...literals].map((match) => match[1] ?? '').filter((name) => pascalCase(name) in lucide);
}

describe('lucide icon registry', () => {
  it('carries an entry for every icon name the source can hand to Icon', () => {
    const missing = new Map<string, string[]>();

    for (const file of sourceFiles(SOURCE_ROOT)) {
      const absent = lucideNamesUsedIn(readFileSync(file, 'utf8')).filter((name) => !(name in LUCIDE_ICONS));
      if (absent.length > 0) missing.set(relative(SOURCE_ROOT, file), [...new Set(absent)]);
    }

    expect(Object.fromEntries(missing)).toEqual({});
  });

  it('never pulls the whole icon set in through a namespace import', () => {
    const sources = sourceFiles(SOURCE_ROOT).map((file) => readFileSync(file, 'utf8'));

    expect(sources.filter((source) => /import \* as \w+ from 'lucide-vue-next'/.test(source))).toEqual([]);
  });
});

describe('Icon', () => {
  it('renders a registered lucide glyph', () => {
    expect(
      mount(Icon, { props: { name: 'plus' } })
        .find('svg')
        .exists(),
    ).toBe(true);
  });

  it('renders a custom glyph that lucide has no equivalent for', () => {
    expect(
      mount(Icon, { props: { name: 'atlas-glyph' } })
        .find('svg')
        .exists(),
    ).toBe(true);
  });

  it('renders nothing for an unknown name', () => {
    expect(
      mount(Icon, { props: { name: 'not-an-icon-at-all' } })
        .find('svg')
        .exists(),
    ).toBe(false);
  });
});
