import { describe, expect, it } from 'vitest';
import { findApiLiterals, findViolations, loadDocumentKeys, loadProductionSrc } from './v1PathLiteralGuard';

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
});
