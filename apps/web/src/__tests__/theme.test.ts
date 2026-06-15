import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const srcDir = resolve(__dirname, '..');
const tokensPath = resolve(srcDir, 'theme/tokens.css');
const indexPath = resolve(srcDir, 'theme/index.css');

describe('theme tokens (REQ-W29)', () => {
  it('tokens.css defines --c-background', () => {
    const content = readFileSync(tokensPath, 'utf-8');
    expect(content).toContain('--c-background');
  });

  it('tokens.css defines core palette variables without hardcoded hex in @theme', () => {
    const content = readFileSync(tokensPath, 'utf-8');
    expect(content).toContain('--c-panel');
    expect(content).toContain('--c-primary');
    expect(content).toContain('--c-danger');
  });

  it('index.css maps --color-background to var(--c-background)', () => {
    const content = readFileSync(indexPath, 'utf-8');
    expect(content).toContain('--color-background');
    expect(content).toContain('var(--c-background)');
  });

  it('index.css @theme block uses var() references not hardcoded hex', () => {
    const content = readFileSync(indexPath, 'utf-8');
    const themeBlock = content.substring(content.indexOf('@theme'));
    expect(themeBlock).not.toMatch(/#[0-9A-Fa-f]{3,6}\b/);
  });
});
