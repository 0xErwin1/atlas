import type { Client } from 'openapi-fetch';
import type { paths as actaPaths } from './generated/acta.d.ts';
import type { paths as custosPaths } from './generated/custos.d.ts';
import type { paths as platformPaths } from './generated/platform.d.ts';
import { wrappedClient } from './wrapper';

/**
 * The one hand-written component list in this slice (design D4.4). It is
 * audited, not trusted: `components.test.ts` asserts it equals the set of
 * `.d.ts` basenames the generator emitted, in both directions.
 */
export const DECLARED_COMPONENTS = ['acta', 'custos', 'platform'] as const;

export const acta = wrappedClient as unknown as Client<actaPaths>;
export const custos = wrappedClient as unknown as Client<custosPaths>;
export const platform = wrappedClient as unknown as Client<platformPaths>;
