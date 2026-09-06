import type { paths as actaPaths } from './generated/acta.d.ts';
import type { paths as custosPaths } from './generated/custos.d.ts';
import type { paths as platformPaths } from './generated/platform.d.ts';
import type { paths } from './types.d.ts';

/**
 * v2-e11-s6 PR2, design D2 — a compile-time-only proof that the
 * per-component split (design D1) is lossless against the flat generated
 * module: `Split` must cover every flat key and invent no key of its own.
 * Neither assertion carries runtime behavior; a failure here means
 * `gen-types` either dropped or duplicated a path key across modules.
 */
type Split = actaPaths & custosPaths & platformPaths;

const _splitCoversFlat: Split = {} as paths;
const _flatCoversSplit: paths = {} as Split;
