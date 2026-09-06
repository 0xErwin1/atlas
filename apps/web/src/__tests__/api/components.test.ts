import { readdirSync } from 'node:fs';
import { basename } from 'node:path';
import { describe, expect, it } from 'vitest';
import { GENERATED_DIR } from '../../../scripts/gen-types.mjs';
import { DECLARED_COMPONENTS } from '../../api/components';

describe('components — D4.4 binding audit', () => {
  it('declares exactly the components the generator emitted, in both directions', () => {
    const generated = new Set(
      readdirSync(GENERATED_DIR)
        .filter((entry) => entry.endsWith('.d.ts'))
        .map((entry) => basename(entry, '.d.ts')),
    );
    const declared = new Set(DECLARED_COMPONENTS);

    expect(declared).toEqual(generated);
  });
});
