import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Assert-only pins for E11-S7 U1.4 (SHELL-NAV-1, -3, -4; design D8/E,
 * §9 F4). Each case pins a fact that already holds in the shipped
 * codebase — there is no red-to-green transition here, per D-S7-3.
 *
 * SHELL-NAV-3 does not hold as shipped: the requirement says the workspace
 * selector is rendered by Acta inside Acta's own area, but
 * `WorkspaceSwitcher.vue` is rendered unconditionally by the shell's own
 * `ContextSidebar.vue`, on every area including Settings. That case below
 * pins the shipped placement and names this divergence explicitly, rather
 * than asserting the requirement's wording against a fixture. It is
 * recorded as an epic-closeout follow-up (design §9 F4), not built here.
 */

const SOURCE_ROOT = resolve(__dirname, '..');

function readSource(relativePath: string): string {
  return readFileSync(resolve(SOURCE_ROOT, relativePath), 'utf-8');
}

describe('shell/nav assert-only pins', () => {
  it('SHELL-NAV-1: the auth store holds principal and credential only, never workspace context', () => {
    const authSource = readSource('stores/auth.ts');

    expect(authSource).not.toMatch(/activeWorkspace/);
    expect(authSource).not.toMatch(/workspaceId/);

    const returned = authSource.slice(authSource.indexOf('return {', authSource.lastIndexOf('return {')));
    expect(returned).toContain('user');
    expect(returned).toContain('sessionActor');

    const workspaceSource = readSource('stores/workspace.ts');
    expect(workspaceSource).toMatch(/activeWorkspaceSlug/);
  });

  it('SHELL-NAV-4: CommandPalette.vue imports no global index and renders only search hits handed to it', () => {
    const paletteSource = readSource('components/search/CommandPalette.vue');

    expect(paletteSource).not.toMatch(/globalIndex/i);
    expect(paletteSource).not.toMatch(/import .*Index.* from/);
    expect(paletteSource).toMatch(/useSearch\(props\.ws\)/);
    expect(paletteSource).toMatch(/store\.results/);
  });

  it('SHELL-NAV-3 asserted as shipped (divergence named, not silently passed): WorkspaceSwitcher.vue is rendered unconditionally by ContextSidebar.vue, not by Acta', () => {
    const sidebarSource = readSource('components/shell/ContextSidebar.vue');

    expect(sidebarSource).toMatch(
      /import WorkspaceSwitcher from '@\/components\/shell\/WorkspaceSwitcher\.vue'/,
    );
    expect(sidebarSource).toMatch(/<div class="flex-1 min-w-0">\s*<WorkspaceSwitcher \/>\s*<\/div>/);
  });
});
