import { describe, expect, it } from 'vitest';
import { atlasMarkdownThemeSpec } from '@/components/editor/theme';

type Rule = Record<string, string>;

function rule(selector: string): Rule {
  const found = atlasMarkdownThemeSpec[selector];
  if (found === undefined) throw new Error(`missing rule: ${selector}`);
  return found;
}

// Prose is read, so it is set in the UI family. The data family is reserved for
// the constructs that are compared column by column: code, tables and the
// language badge.
const CODE_RULES = [
  '.cm-atlas-code',
  '.cm-atlas-fenced',
  '.cm-atlas-lang',
  '.cm-atlas-table',
  '.cm-lineNumbers .cm-gutterElement',
];

describe('markdown editor theme', () => {
  it('sets prose in the UI family', () => {
    expect(rule('&').fontFamily).toBe('var(--font-ui)');
    expect(rule('.cm-scroller').fontFamily).toBe('var(--font-ui)');
  });

  it.each(CODE_RULES)('keeps %s in the data family', (selector) => {
    expect(rule(selector).fontFamily).toBe('var(--font-mono)');
  });

  it('declares no rounded corner anywhere', () => {
    for (const [selector, declarations] of Object.entries(atlasMarkdownThemeSpec)) {
      for (const [property, value] of Object.entries(declarations)) {
        if (property.startsWith('borderRadius') || property.startsWith('border-radius')) {
          throw new Error(`${selector} declares ${property}: ${value}`);
        }
      }
    }
    expect(true).toBe(true);
  });
});
