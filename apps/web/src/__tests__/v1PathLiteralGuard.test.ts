import { describe, expect, it } from 'vitest';
import {
  findApiLiterals,
  findViolations,
  loadDocumentKeys,
  loadDocumentOwners,
  loadProductionSrc,
} from './v1PathLiteralGuard';

describe('v1PathLiteralGuard — probe self-test', () => {
  const documentKeys = new Set(['/api/v2/acta/workspaces/{}/tasks', '/api/v2/custos/auth/me']);

  it('flags a V1-form literal by file name', () => {
    const violations = findViolations(
      new Map([['src/scratch/a.ts', "const p = '/api/workspaces/{ws}/tasks';"]]),
      documentKeys,
      new Set(),
    );

    expect(violations).toEqual([
      {
        file: 'src/scratch/a.ts',
        literal: '/api/workspaces/{ws}/tasks',
        reason: 'does not start with /api/v2/',
      },
    ]);
  });

  it('flags a V2-shaped literal naming a component the document does not declare', () => {
    const violations = findViolations(
      new Map([['src/scratch/b.ts', "const p = '/api/v2/custos/workspaces/{ws}/tasks';"]]),
      documentKeys,
      new Set(),
    );

    expect(violations).toEqual([
      {
        file: 'src/scratch/b.ts',
        literal: '/api/v2/custos/workspaces/{ws}/tasks',
        reason: 'not a key in openapi.json',
      },
    ]);
  });

  it('passes a real V2 document key', () => {
    const violations = findViolations(
      new Map([['src/scratch/c.ts', "const p = '/api/v2/acta/workspaces/{ws}/tasks';"]]),
      documentKeys,
      new Set(),
    );

    expect(violations).toEqual([]);
  });

  it('does not flag prose inside a comment', () => {
    const violations = findViolations(
      new Map([['src/scratch/d.ts', '// see `/api/workspaces/{ws}/tasks` for the old shape\nexport {};']]),
      documentKeys,
      new Set(),
    );

    expect(violations).toEqual([]);
  });

  it('still flags a V1 literal that follows an apostrophe inside a comment', () => {
    const violations = findViolations(
      new Map([['src/scratch/g.ts', "// the project's old shape\nconst p = '/api/workspaces/{ws}/tasks';"]]),
      documentKeys,
      new Set(),
    );

    expect(violations).toEqual([
      {
        file: 'src/scratch/g.ts',
        literal: '/api/workspaces/{ws}/tasks',
        reason: 'does not start with /api/v2/',
      },
    ]);
  });

  it('still flags a V1 literal that follows a quote inside a regex literal', () => {
    const violations = findViolations(
      new Map([
        ['src/scratch/h.ts', "const q = s.replace(/'/g, '');\nconst p = '/api/workspaces/{ws}/tasks';"],
      ]),
      documentKeys,
      new Set(),
    );

    expect(violations).toEqual([
      {
        file: 'src/scratch/h.ts',
        literal: '/api/workspaces/{ws}/tasks',
        reason: 'does not start with /api/v2/',
      },
    ]);
  });

  it('respects the allowlist by exact (file, literal) pair', () => {
    const violations = findViolations(
      new Map([['src/scratch/e.ts', "const p = '/api/';"]]),
      documentKeys,
      new Set(['src/scratch/e.ts::/api/']),
    );

    expect(violations).toEqual([]);
  });

  it('flags a stale allowlist entry whose literal no longer exists in the scanned tree', () => {
    const violations = findViolations(
      new Map([['src/scratch/f.ts', "const p = '/api/v2/custos/auth/me';"]]),
      documentKeys,
      new Set(['src/scratch/f.ts::/api/']),
    );

    expect(violations).toEqual([
      {
        file: 'src/scratch/f.ts',
        literal: '/api/',
        reason: 'stale allowlist entry: no such literal in the scanned tree',
      },
    ]);
  });
});

describe('v1PathLiteralGuard — ownership check (D5.3)', () => {
  const documentKeys = new Set(['/api/v2/acta/workspaces/{}/tasks', '/api/v2/custos/workspaces/{}/tasks']);
  const documentOwners = new Map([
    ['/api/v2/acta/workspaces/{}/tasks', 'acta'],
    ['/api/v2/custos/workspaces/{}/tasks', 'custos'],
  ]);

  it('passes a document key called through its own component', () => {
    const violations = findViolations(
      new Map([['src/scratch/i.ts', "acta.get('/api/v2/acta/workspaces/{ws}/tasks');"]]),
      documentKeys,
      new Set(),
      documentOwners,
    );

    expect(violations).toEqual([]);
  });

  it("flags a document key called through another component's sub-client", () => {
    const violations = findViolations(
      new Map([['src/scratch/j.ts', "custos.get('/api/v2/acta/workspaces/{ws}/tasks');"]]),
      documentKeys,
      new Set(),
      documentOwners,
    );

    expect(violations).toEqual([
      {
        file: 'src/scratch/j.ts',
        literal: '/api/v2/acta/workspaces/{ws}/tasks',
        reason: 'owned by acta, called through custos',
      },
    ]);
  });

  it('still flags a V2-shaped literal naming a component the document does not declare, when called through a sub-client', () => {
    const violations = findViolations(
      new Map([['src/scratch/k.ts', "custos.get('/api/v2/custos/workspaces/{ws}/tasks');"]]),
      new Set(['/api/v2/acta/workspaces/{}/tasks']),
      new Set(),
      new Map([['/api/v2/acta/workspaces/{}/tasks', 'acta']]),
    );

    expect(violations).toEqual([
      {
        file: 'src/scratch/k.ts',
        literal: '/api/v2/custos/workspaces/{ws}/tasks',
        reason: 'not a key in openapi.json',
      },
    ]);
  });
});

describe('v1PathLiteralGuard — flat-client closure (D5.5)', () => {
  const documentKeys = new Set(['/api/v2/acta/workspaces/{}/tasks']);
  const documentOwners = new Map([['/api/v2/acta/workspaces/{}/tasks', 'acta']]);

  it('flags a document key called through wrappedClient, naming the component to use instead', () => {
    const violations = findViolations(
      new Map([['src/scratch/l.ts', "wrappedClient.get('/api/v2/acta/workspaces/{ws}/tasks');"]]),
      documentKeys,
      new Set(),
      documentOwners,
    );

    expect(violations).toEqual([
      {
        file: 'src/scratch/l.ts',
        literal: '/api/v2/acta/workspaces/{ws}/tasks',
        reason: 'owned by acta, called through wrappedClient',
      },
    ]);
  });

  it('flags a document key called through apiClient, naming the component to use instead', () => {
    const violations = findViolations(
      new Map([['src/scratch/m.ts', "apiClient.get('/api/v2/acta/workspaces/{ws}/tasks');"]]),
      documentKeys,
      new Set(),
      documentOwners,
    );

    expect(violations).toEqual([
      {
        file: 'src/scratch/m.ts',
        literal: '/api/v2/acta/workspaces/{ws}/tasks',
        reason: 'owned by acta, called through apiClient',
      },
    ]);
  });
});

describe('v1PathLiteralGuard — document owners (D5.3)', () => {
  it('maps every path key to its x-atlas-component, with 142 entries and known owners', () => {
    const owners = loadDocumentOwners();

    expect(owners.size).toBe(142);
    expect(owners.get('/health')).toBe('platform');
    expect(owners.get('/api/v2/acta/admin/status-templates')).toBe('acta');
    expect(owners.get('/api/v2/custos/activate/{}')).toBe('custos');
  });
});

describe('v1PathLiteralGuard — production src', () => {
  it('has zero violations across every non-generated, non-test production file', async () => {
    const files = await loadProductionSrc();
    const documentKeys = loadDocumentKeys();

    const violations = findViolations(files, documentKeys);

    expect(violations).toEqual([]);
  });

  it('scans the real tree, not a near-empty walk', async () => {
    const files = await loadProductionSrc();

    expect(files.size).toBeGreaterThan(200);
    expect(findApiLiterals(files.get('src/platform/browser.ts') ?? '')).not.toHaveLength(0);
  });

  it('covers apps/web/scripts/ (D5.4) so the generator is not a blind spot', async () => {
    const files = await loadProductionSrc();

    expect(files.has('scripts/gen-types.mjs')).toBe(true);
  });
});
