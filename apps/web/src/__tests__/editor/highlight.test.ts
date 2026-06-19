import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { atlasHighlight } from '@/components/editor/highlight';

const tokensPath = resolve(__dirname, '../../theme/tokens.css');

/** The syntax tags the editor colours; each must resolve to a CSS variable. */
const SYNTAX_TOKENS = [
  '--c-syntax-keyword',
  '--c-syntax-string',
  '--c-syntax-comment',
  '--c-syntax-number',
  '--c-syntax-function',
  '--c-syntax-type',
  '--c-syntax-operator',
];

describe('editor syntax highlight (E07 editor-syntax-highlighting)', () => {
  it('exports atlasHighlight as a CodeMirror extension', () => {
    expect(atlasHighlight).toBeDefined();
  });

  it('tokens.css defines the syntax colour set', () => {
    const css = readFileSync(tokensPath, 'utf-8');
    for (const token of SYNTAX_TOKENS) {
      expect(css).toContain(token);
    }
  });

  it('defines the syntax colour set for both dark and light themes', () => {
    const css = readFileSync(tokensPath, 'utf-8');
    const lightStart = css.indexOf("[data-theme='light']");
    expect(lightStart).toBeGreaterThan(-1);

    const darkBlock = css.slice(0, lightStart);
    const lightBlock = css.slice(lightStart);

    for (const token of SYNTAX_TOKENS) {
      expect(darkBlock).toContain(token);
      expect(lightBlock).toContain(token);
    }
  });
});
