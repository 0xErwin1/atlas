/**
 * Converts a title string to a URL slug, matching the server-side slugification
 * logic used by Atlas document routes.
 *
 * Rules:
 * - Lowercase all characters
 * - Replace any non-ASCII-alphanumeric character with a hyphen
 * - Collapse consecutive hyphens into one
 * - Trim leading and trailing hyphens
 */
export function slugify(title: string): string {
  return title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}
