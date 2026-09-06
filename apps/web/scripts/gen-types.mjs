#!/usr/bin/env node

/**
 * v2-e11-s6 PR2, design D1/D2/D4 — filters the one composed
 * `apps/web/openapi.json` document into one generated type module per
 * component present in it, plus the existing flat `src/api/types.d.ts`.
 *
 * The component set is derived from the document's own
 * `x-atlas-component` extension (never a hand-maintained list, never
 * `tags` — design §0.2/§0.3), the document is read exactly once
 * (INV-NO-SECOND-DOCUMENT), and the whole pipeline is a pure function of
 * that one value (INV-REPRODUCIBLE-GENERATION).
 */

import { readFileSync } from 'node:fs';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import openapiTS, { astToString, COMMENT_HEADER } from 'openapi-typescript';

// Deliberately not `new URL('.', import.meta.url)`: under Vitest's jsdom
// environment the global `URL` constructor resolves a relative `'.'`
// against jsdom's own `http://localhost` base instead of the `file:` base,
// silently discarding the real directory. `dirname` over the resolved
// file path has no such ambiguity.
export const SCRIPTS_DIR = dirname(fileURLToPath(import.meta.url));
export const WEB_ROOT = join(SCRIPTS_DIR, '..');
export const OPENAPI_PATH = join(WEB_ROOT, 'openapi.json');
export const GENERATED_DIR = join(WEB_ROOT, 'src', 'api', 'generated');
export const FLAT_TYPES_PATH = join(WEB_ROOT, 'src', 'api', 'types.d.ts');

const OPERATION_METHODS = ['get', 'put', 'post', 'delete', 'options', 'head', 'patch', 'trace'];
const SCHEMA_REF_RE = /^#\/components\/schemas\/([^/]+)$/;

/**
 * Groups the document's operations by their `x-atlas-component` extension.
 * A path key survives into a component's subdocument iff at least one of
 * its operations is owned by that component; every non-owned operation on
 * a shared key is deleted from the copy, and the key's `parameters`/
 * `servers` are carried along (design D1.2). The component set itself is a
 * `Set` derived from the operations, never a literal list, so an absent
 * component produces no key and an unexpected one produces its own
 * (design D4).
 */
export function splitByComponent(doc) {
  const paths = doc.paths ?? {};
  const componentIds = new Set();

  for (const pathItem of Object.values(paths)) {
    for (const method of OPERATION_METHODS) {
      const owner = pathItem[method]?.['x-atlas-component'];
      if (owner) {
        componentIds.add(owner);
      }
    }
  }

  const result = {};

  for (const componentId of componentIds) {
    const retainedPaths = {};

    for (const [pathKey, pathItem] of Object.entries(paths)) {
      const ownedMethods = OPERATION_METHODS.filter(
        (method) => pathItem[method]?.['x-atlas-component'] === componentId,
      );
      if (ownedMethods.length === 0) {
        continue;
      }

      const retainedPathItem = {};
      if (pathItem.parameters !== undefined) {
        retainedPathItem.parameters = pathItem.parameters;
      }
      if (pathItem.servers !== undefined) {
        retainedPathItem.servers = pathItem.servers;
      }
      for (const method of ownedMethods) {
        retainedPathItem[method] = pathItem[method];
      }

      retainedPaths[pathKey] = retainedPathItem;
    }

    result[componentId] = {
      ...doc,
      paths: retainedPaths,
    };
  }

  return result;
}

function collectSchemaRefs(node, refs) {
  if (Array.isArray(node)) {
    for (const item of node) {
      collectSchemaRefs(item, refs);
    }
    return;
  }

  if (node && typeof node === 'object') {
    for (const [key, value] of Object.entries(node)) {
      if (key === '$ref' && typeof value === 'string') {
        const match = SCHEMA_REF_RE.exec(value);
        if (match) {
          refs.add(match[1]);
        }
        continue;
      }
      collectSchemaRefs(value, refs);
    }
  }
}

/**
 * The dangling-ref self-test (design D1.3, mirroring
 * `openapi_zero_drift.rs`'s `dangling_schema_refs`/`collect_schema_refs`):
 * every `$ref` reachable from a subdocument's `paths` or its own retained
 * `components.schemas` must resolve to a schema present in that same
 * subdocument. Run before generation, so a closure walk that returns
 * everything or nothing is caught before it reaches `openapiTS`.
 */
export function assertNoDanglingRefs(subdoc) {
  const schemas = subdoc.components?.schemas ?? {};
  const refs = new Set();
  collectSchemaRefs(subdoc.paths, refs);
  collectSchemaRefs(schemas, refs);

  for (const name of refs) {
    if (!(name in schemas)) {
      throw new Error(`dangling $ref: #/components/schemas/${name} does not resolve in this subdocument`);
    }
  }
}

/**
 * Prunes `components.schemas` to the transitive `$ref` closure reachable
 * from the subdocument's own paths. `securitySchemes`, `parameters`,
 * `responses` and the document-level `info`/`openapi`/`servers`/
 * `security`/`tags` fields are carried whole — they are small, and
 * filtering them would add a way to be wrong for no emitted-type benefit
 * (design D1.3). Shared schemas such as `Problem`/`Page<…>` end up
 * duplicated across the modules that reference them, deliberately
 * (design D1.4).
 */
export function pruneSchemas(subdoc) {
  const allSchemas = subdoc.components?.schemas ?? {};
  const reachable = new Set();
  const worklist = [];
  collectSchemaRefs(subdoc.paths, { add: (name) => worklist.push(name) });

  while (worklist.length > 0) {
    const name = worklist.pop();
    if (reachable.has(name)) {
      continue;
    }
    reachable.add(name);

    const schema = allSchemas[name];
    if (schema === undefined) {
      continue;
    }
    const nested = new Set();
    collectSchemaRefs(schema, nested);
    for (const ref of nested) {
      if (!reachable.has(ref)) {
        worklist.push(ref);
      }
    }
  }

  const prunedSchemas = {};
  for (const name of reachable) {
    if (allSchemas[name] !== undefined) {
      prunedSchemas[name] = allSchemas[name];
    }
  }

  const pruned = {
    ...subdoc,
    components: {
      ...subdoc.components,
      schemas: prunedSchemas,
    },
  };

  assertNoDanglingRefs(pruned);
  return pruned;
}

/**
 * The composed, pure pipeline: split the document by component, prune
 * each subdocument's schema closure, and run `openapiTS`/`astToString`
 * once per present component plus once on the whole document for the
 * flat module (design D2 — four emits, not three). Pure given `doc`: no
 * filesystem access and no timestamp in the emitted text, so
 * INV-REPRODUCIBLE-GENERATION holds structurally rather than by
 * convention.
 */
export async function generateAll(doc) {
  const split = splitByComponent(doc);
  const outputs = {};

  for (const componentId of Object.keys(split).sort()) {
    const pruned = pruneSchemas(split[componentId]);
    const ast = await openapiTS(pruned);
    outputs[componentId] = `${COMMENT_HEADER}${astToString(ast)}`;
  }

  const flatAst = await openapiTS(doc);
  outputs.flat = `${COMMENT_HEADER}${astToString(flatAst)}`;

  return outputs;
}

/**
 * Writes `generateAll`'s output to disk. `generatedDir` is removed and
 * rewritten wholesale before each write, so a component that disappears
 * from the document leaves no stale module behind on an incremental run
 * (design D4.3) — that is what INV-ABSENT-COMPONENT-NO-MODULE means on a
 * developer's machine, as opposed to in a fixture.
 */
export async function writeGenerated(doc, { generatedDir, flatPath }) {
  const outputs = await generateAll(doc);

  await rm(generatedDir, { recursive: true, force: true });
  await mkdir(generatedDir, { recursive: true });

  await Promise.all(
    Object.entries(outputs)
      .filter(([componentId]) => componentId !== 'flat')
      .map(([componentId, content]) => writeFile(join(generatedDir, `${componentId}.d.ts`), content, 'utf8')),
  );

  await writeFile(flatPath, outputs.flat, 'utf8');

  return outputs;
}

async function main() {
  const doc = JSON.parse(readFileSync(OPENAPI_PATH, 'utf8'));
  await writeGenerated(doc, { generatedDir: GENERATED_DIR, flatPath: FLAT_TYPES_PATH });
}

const isMainModule = process.argv[1] === fileURLToPath(import.meta.url);
if (isMainModule) {
  await main();
}
