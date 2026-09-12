import { existsSync, readFileSync } from 'node:fs';
import { mkdir, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { OpenAPI3, PathItemObject } from 'openapi-typescript';
import { describe, expect, it } from 'vitest';
import {
  assertNoDanglingRefs,
  FLAT_TYPES_PATH,
  GENERATED_DIR,
  generateAll,
  OPENAPI_PATH,
  pruneSchemas,
  splitByComponent,
  WEB_ROOT,
  writeGenerated,
} from '../../scripts/gen-types.mjs';

/**
 * v2-e11-s6 PR2, design D1/D2/D4 — proves `gen-types`'s per-component split
 * is derived from the document's own `x-atlas-component` stamps rather than
 * declared, that its schema closure has no dangling `$ref`, that the emit
 * step is total over the generated directory, and that the whole pipeline
 * is deterministic. All fixtures clone `apps/web/openapi.json` rather than
 * hand-authoring a second document (INV-NO-SECOND-DOCUMENT), so every
 * component key and `paths`/`components` field asserted against below is
 * known to be present in the real document.
 */

const GENERATION_TIMEOUT_MS = 60_000;

const OPERATION_METHODS = ['get', 'put', 'post', 'delete', 'options', 'head', 'patch', 'trace'] as const;

function required<T>(value: T | undefined, description: string): T {
  if (value === undefined) {
    throw new Error(`expected ${description} to be present`);
  }
  return value;
}

function loadDocument(): OpenAPI3 {
  return JSON.parse(readFileSync(OPENAPI_PATH, 'utf8'));
}

function countOperations(subdoc: OpenAPI3): number {
  let count = 0;
  for (const pathItem of Object.values(subdoc.paths ?? {}) as PathItemObject[]) {
    for (const method of OPERATION_METHODS) {
      if (pathItem[method] !== undefined) {
        count += 1;
      }
    }
  }
  return count;
}

function collectRefNames(node: unknown, refs: Set<string>): void {
  if (Array.isArray(node)) {
    for (const item of node) {
      collectRefNames(item, refs);
    }
    return;
  }
  if (node && typeof node === 'object') {
    for (const [key, value] of Object.entries(node)) {
      if (key === '$ref' && typeof value === 'string') {
        const match = /^#\/components\/schemas\/([^/]+)$/.exec(value);
        const schemaName = match?.[1];
        if (schemaName) {
          refs.add(schemaName);
        }
        continue;
      }
      collectRefNames(value, refs);
    }
  }
}

describe('splitByComponent', () => {
  it('derives exactly {acta, custos, platform} with the measured per-component operation counts', () => {
    const doc = loadDocument();
    const split = splitByComponent(doc);

    expect(Object.keys(split).sort()).toEqual(['acta', 'custos', 'platform']);
    expect(countOperations(required(split.acta, 'acta in split'))).toBe(171);
    expect(countOperations(required(split.custos, 'custos in split'))).toBe(38);
    expect(countOperations(required(split.platform, 'platform in split'))).toBe(7);
  });

  it('produces a fourth key for a synthetic component stamp, proving the split is derived not declared', () => {
    const doc = loadDocument();
    const mutated = structuredClone(doc);
    required(mutated.paths, 'paths on the cloned document')['/api/v2/search/probe'] = {
      get: {
        operationId: 'searchProbe',
        responses: { 200: { description: 'ok' } },
        'x-atlas-component': 'search',
      },
    };

    const split = splitByComponent(mutated);
    const searchDoc = required(split.search, 'search in split');

    expect(Object.keys(split).sort()).toEqual(['acta', 'custos', 'platform', 'search']);
    expect(countOperations(searchDoc)).toBe(1);
    expect(Object.keys(searchDoc.paths ?? {})).toEqual(['/api/v2/search/probe']);
  });

  it('produces no custos key when every custos operation is removed, leaving acta and platform unaffected', () => {
    const doc = loadDocument();
    const mutated = structuredClone(doc);

    for (const pathItem of Object.values(mutated.paths ?? {}) as PathItemObject[]) {
      for (const method of OPERATION_METHODS) {
        if (pathItem[method]?.['x-atlas-component'] === 'custos') {
          delete pathItem[method];
        }
      }
    }

    const split = splitByComponent(mutated);

    expect(Object.keys(split).sort()).toEqual(['acta', 'platform']);
    expect(countOperations(required(split.acta, 'acta in split'))).toBe(171);
    expect(countOperations(required(split.platform, 'platform in split'))).toBe(7);
  });
});

describe('pruneSchemas', () => {
  it('produces a schema closure where every $ref resolves within the same subdocument', () => {
    const doc = loadDocument();
    const split = splitByComponent(doc);

    for (const componentId of Object.keys(split)) {
      const pruned = pruneSchemas(required(split[componentId], `${componentId} in split`));
      const refs = new Set<string>();
      collectRefNames(pruned.paths, refs);
      collectRefNames(pruned.components?.schemas, refs);

      for (const ref of refs) {
        expect(pruned.components?.schemas).toHaveProperty(ref);
      }
    }
  });

  it('rejects a fabricated dangling $ref, naming it', () => {
    const doc = loadDocument();
    const split = splitByComponent(doc);
    const pruned = pruneSchemas(required(split.acta, 'acta in split'));

    const withDanglingRef = structuredClone(pruned);
    const schemas = required(
      required(withDanglingRef.components, 'components on the cloned subdocument').schemas,
      'schemas on the cloned subdocument',
    );
    schemas.__FabricatedDangling = {
      type: 'object',
      properties: {
        missing: { $ref: '#/components/schemas/DoesNotExist' },
      },
    };

    expect(() => assertNoDanglingRefs(withDanglingRef)).toThrowError(/DoesNotExist/);
  });
});

describe('directory totality', () => {
  it(
    'wipes a stale module and writes exactly the present components, in both directions',
    async () => {
      const doc = loadDocument();
      const stalePath = join(GENERATED_DIR, 'search.d.ts');
      await mkdir(GENERATED_DIR, { recursive: true });
      await writeFile(stalePath, '// stale', 'utf8');
      expect(existsSync(stalePath)).toBe(true);

      await writeGenerated(doc, { generatedDir: GENERATED_DIR, flatPath: FLAT_TYPES_PATH });

      expect(existsSync(stalePath)).toBe(false);

      const entries = await readdir(GENERATED_DIR);
      const basenames = entries.map((entry) => entry.replace(/\.d\.ts$/, '')).sort();

      expect(basenames).toEqual(['acta', 'custos', 'platform']);
    },
    GENERATION_TIMEOUT_MS,
  );
});

describe('reproducibility', () => {
  it(
    'produces byte-identical output across two in-process runs, with no timestamp or absolute path',
    async () => {
      const doc = loadDocument();

      const first = await generateAll(doc);
      const second = await generateAll(doc);

      expect(Object.keys(first).sort()).toEqual(['acta', 'custos', 'flat', 'platform']);
      expect(second).toEqual(first);

      const isoLikeRe = /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/;
      for (const content of Object.values(first)) {
        expect(content).not.toMatch(isoLikeRe);
        expect(content.includes(WEB_ROOT)).toBe(false);
      }
    },
    GENERATION_TIMEOUT_MS,
  );
});
