import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const baseCss = readFileSync(resolve(process.cwd(), 'src/theme/base.css'), 'utf8');
const formField = readFileSync(resolve(process.cwd(), 'src/components/ui/FormField.vue'), 'utf8');

function ruleBody(source: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = source.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  expect(match, `missing rule: ${selector}`).not.toBeNull();
  return match?.[1] ?? '';
}

describe('shared field focus rings', () => {
  it('draws the tokenized external ring on standalone controls without changing geometry', () => {
    const focusedControls = ruleBody(
      baseCss,
      'input:focus-visible,\ntextarea:focus-visible,\nselect:focus-visible',
    );

    expect(focusedControls).toContain('box-shadow: var(--shadow-ring)');
    expect(focusedControls).not.toContain('inset');
    expect(focusedControls).not.toContain('border-width');
  });

  it('draws one ring around the complete FormField control', () => {
    const focusedBox = ruleBody(formField, '.atl-field-box:focus-within');
    const focusedInput = ruleBody(formField, '.atl-field-input:focus-visible');

    expect(focusedBox).toContain('box-shadow: var(--shadow-ring)');
    expect(focusedBox).not.toContain('border-color');
    expect(focusedInput).toContain('box-shadow: none');
  });
});
