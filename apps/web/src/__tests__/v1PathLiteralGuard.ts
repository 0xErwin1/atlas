import { readFileSync } from 'node:fs';
import { readdir } from 'node:fs/promises';
import { join, relative } from 'node:path';

/**
 * D4.5 layer 2: the source guard for the two sites `vue-tsc` cannot see
 * (`EventSource`, the 401-suppression `Set`) and for any future untyped site.
 * `vue-tsc` only proves the 221+ typed `wrappedClient.VERB('/api/v2/...')`
 * call sites, because a V1 key is no longer a member of the generated `paths`
 * type. A raw string passed to `fetch`/`EventSource`/`Set` carries no such
 * type, so this guard re-derives the same two facts by reading the source
 * text and the regenerated `openapi.json` directly.
 */

export const WEB_ROOT = join(__dirname, '..', '..');
const SRC_ROOT = join(WEB_ROOT, 'src');
const OPENAPI_PATH = join(WEB_ROOT, 'openapi.json');

// A string literal (single/double/backtick-quoted) whose content is an API
// path: it starts with `/api` followed by a path separator, a query marker,
// or nothing else.
const STRING_LITERAL_RE = /(['"`])((?:\\.|(?!\1)[\s\S])*?)\1/g;
const API_PATH_RE = /^\/api(\/|$|\?)/;
const PLACEHOLDER_RE = /\{[^}]*\}|\$\{[^}]*\}/g;

export interface Violation {
  file: string;
  literal: string;
  reason: string;
}

const REGEX_PRECEDING_PUNCTUATION = new Set([
  '(',
  ',',
  '=',
  ':',
  '[',
  '!',
  '&',
  '|',
  '?',
  '{',
  '}',
  ';',
  '+',
  '-',
  '*',
  '%',
  '<',
  '>',
  '~',
  '^',
]);
const REGEX_PRECEDING_KEYWORDS = new Set([
  'return',
  'typeof',
  'case',
  'do',
  'else',
  'in',
  'instanceof',
  'new',
  'throw',
  'void',
  'yield',
  'await',
  'of',
]);

/**
 * True when a `/` at the current position opens a regex literal rather than a
 * division: the previous significant token is an operator, an opener, or a
 * keyword that cannot be followed by an operand. `out` is the masked prefix,
 * so comments already read as whitespace.
 */
function regexCanStartAfter(out: string[]): boolean {
  let j = out.length - 1;
  while (j >= 0 && /\s/.test(out[j] ?? '')) j -= 1;
  if (j < 0) return true;

  const prev = out[j] ?? '';
  if (REGEX_PRECEDING_PUNCTUATION.has(prev)) return true;
  if (!/[A-Za-z_$]/.test(prev)) return false;

  let start = j;
  while (start > 0 && /[A-Za-z0-9_$]/.test(out[start - 1] ?? '')) start -= 1;
  return REGEX_PRECEDING_KEYWORDS.has(out.slice(start, j + 1).join(''));
}

/**
 * Blanks a regex literal starting at `text[i]` (the opening `/`) into `out`,
 * honouring escapes and character classes where `/` does not terminate, then
 * its flags. Returns the index just past the literal. A newline ends the scan
 * early so a mis-detected division cannot swallow the rest of the file.
 */
function maskRegexLiteral(text: string, i: number, out: string[]): number {
  let j = i + 1;
  let inClass = false;
  out.push(' ');

  while (j < text.length) {
    const ch = text.charAt(j);
    if (ch === '\n') return j;

    if (ch === '\\') {
      out.push(' ', ' ');
      j += 2;
      continue;
    }

    out.push(' ');
    j += 1;

    if (inClass) {
      if (ch === ']') inClass = false;
      continue;
    }
    if (ch === '[') {
      inClass = true;
      continue;
    }
    if (ch === '/') break;
  }

  while (j < text.length && /[a-z]/.test(text.charAt(j))) {
    out.push(' ');
    j += 1;
  }

  return j;
}

/**
 * Blanks line comments, block comments, prose inside JSDoc, and regex
 * literals so a doc comment's example path (e.g. "`/api/…`"), an apostrophe
 * in prose, or a quote inside a regex is never read as (part of) a code
 * literal. String literals pass through verbatim, so the result keeps every
 * byte offset of the input. Mirrors the masking technique the PR4 rewrite
 * script used.
 */
function maskComments(text: string): string {
  const out: string[] = [];
  let i = 0;
  let inLineComment = false;
  let inBlockComment = false;
  let inString: string | null = null;

  while (i < text.length) {
    const ch = text.charAt(i);
    const next = text.charAt(i + 1);

    if (inLineComment) {
      out.push(ch === '\n' ? '\n' : ' ');
      if (ch === '\n') inLineComment = false;
      i += 1;
      continue;
    }

    if (inBlockComment) {
      if (ch === '*' && next === '/') {
        out.push(' ', ' ');
        i += 2;
        inBlockComment = false;
        continue;
      }
      out.push(ch === '\n' ? '\n' : ' ');
      i += 1;
      continue;
    }

    if (inString !== null) {
      out.push(ch);
      if (ch === '\\' && i + 1 < text.length) {
        out.push(text.charAt(i + 1));
        i += 2;
        continue;
      }
      if (ch === inString) inString = null;
      i += 1;
      continue;
    }

    if (ch === "'" || ch === '"' || ch === '`') {
      inString = ch;
      out.push(ch);
      i += 1;
      continue;
    }

    if (ch === '/' && next === '/') {
      inLineComment = true;
      out.push(' ');
      i += 1;
      continue;
    }

    if (ch === '/' && next === '*') {
      inBlockComment = true;
      out.push(' ');
      i += 1;
      continue;
    }

    if (ch === '/' && regexCanStartAfter(out)) {
      i = maskRegexLiteral(text, i, out);
      continue;
    }

    out.push(ch);
    i += 1;
  }

  return out.join('');
}

function normalize(rel: string): string {
  return rel.replace(PLACEHOLDER_RE, '{}');
}

/** Findings for one file's content: every string literal that is an API path,
 * whether it satisfies the guard or not (callers filter for violations). The
 * scan runs over the masked text so a quote inside a comment or a regex can
 * never open a bogus literal that swallows the next real one. */
export function findApiLiterals(content: string): string[] {
  const literals: string[] = [];

  for (const match of maskComments(content).matchAll(STRING_LITERAL_RE)) {
    const literal = match[2] ?? '';
    if (API_PATH_RE.test(literal)) literals.push(literal);
  }

  return literals;
}

export function loadDocumentKeys(): Set<string> {
  const doc = JSON.parse(readFileSync(OPENAPI_PATH, 'utf8')) as { paths: Record<string, unknown> };
  return new Set(Object.keys(doc.paths).map(normalize));
}

/** (file, literal) pairs that are not real routes and are exempted by name,
 * mirroring the Rust guards' allowlist discipline (D2.4). `useApiImageSrc.ts`'s
 * `API_PREFIX` is a generic `startsWith` boundary check, not a route call: it
 * still matches every V2 URL (`/api/v2/...` starts with `/api/`), so it is
 * correct and version-independent as written. */
export const DEFAULT_ALLOWLIST = new Set(['src/composables/useApiImageSrc.ts::/api/']);

/**
 * The allowlist is bidirectional, like the Rust guards': an entry exempts a
 * literal, and an entry whose literal no longer exists in the scanned tree is
 * itself a violation, so a stale exemption cannot outlive the site it covered.
 */
export function findViolations(
  files: Map<string, string>,
  documentKeys: Set<string>,
  allowlist: Set<string> = DEFAULT_ALLOWLIST,
): Violation[] {
  const violations: Violation[] = [];
  const unmatchedAllowlist = new Set(allowlist);

  for (const [file, content] of [...files.entries()].sort(([a], [b]) => a.localeCompare(b))) {
    for (const literal of findApiLiterals(content)) {
      const allowlistKey = `${file}::${literal}`;
      if (allowlist.has(allowlistKey)) {
        unmatchedAllowlist.delete(allowlistKey);
        continue;
      }

      if (!literal.startsWith('/api/v2/')) {
        violations.push({ file, literal, reason: 'does not start with /api/v2/' });
        continue;
      }

      const [relNoQuery] = literal.split('?');
      if (!documentKeys.has(normalize(relNoQuery ?? literal))) {
        violations.push({ file, literal, reason: 'not a key in openapi.json' });
      }
    }
  }

  for (const entry of unmatchedAllowlist) {
    const separator = entry.indexOf('::');
    violations.push({
      file: entry.slice(0, separator),
      literal: entry.slice(separator + 2),
      reason: 'stale allowlist entry: no such literal in the scanned tree',
    });
  }

  return violations.sort((a, b) => a.file.localeCompare(b.file) || a.literal.localeCompare(b.literal));
}

export async function loadProductionSrc(): Promise<Map<string, string>> {
  const files = new Map<string, string>();

  async function walk(dir: string): Promise<void> {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);

      if (entry.isDirectory()) {
        if (entry.name === '__tests__') continue;
        await walk(full);
        continue;
      }

      if (!/\.(ts|vue)$/.test(entry.name)) continue;
      if (entry.name === 'types.d.ts' || entry.name.endsWith('.spec.ts') || entry.name.endsWith('.test.ts')) {
        continue;
      }

      files.set(relative(WEB_ROOT, full), readFileSync(full, 'utf8'));
    }
  }

  await walk(SRC_ROOT);
  return files;
}
