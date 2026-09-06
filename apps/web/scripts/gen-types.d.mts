import type { OpenAPI3 } from 'openapi-typescript';

export const SCRIPTS_DIR: string;
export const WEB_ROOT: string;
export const OPENAPI_PATH: string;
export const GENERATED_DIR: string;
export const FLAT_TYPES_PATH: string;

export function splitByComponent(doc: OpenAPI3): Record<string, OpenAPI3>;

export function assertNoDanglingRefs(subdoc: OpenAPI3): void;

export function pruneSchemas(subdoc: OpenAPI3): OpenAPI3;

export function generateAll(doc: OpenAPI3): Promise<Record<string, string>>;

export function writeGenerated(
  doc: OpenAPI3,
  options: { generatedDir: string; flatPath: string },
): Promise<Record<string, string>>;
