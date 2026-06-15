const MAX_SLUG_LEN = 80;

/**
 * Converts a title string to a URL slug, matching the server-side slugification
 * logic in `crates/atlas_domain/src/slug.rs`.
 *
 * Rules (identical to the Rust implementation):
 * - Lowercase all characters (Unicode-aware via String.prototype.toLowerCase)
 * - Non-alphanumeric runs replaced with a single hyphen (Unicode char.is_alphanumeric() parity:
 *   any character matched by \p{L} or \p{N} is kept; everything else collapses to `-`)
 * - Leading and trailing hyphens trimmed
 * - Truncated to 80 characters; trailing hyphen re-trimmed after truncation
 * - Returns "untitled" when the input is empty or produces only separators
 */
export function slugify(title: string): string {
  const lowered = title.toLowerCase();

  let slug = '';
  let lastWasHyphen = true;

  for (const ch of lowered) {
    if (/\p{L}|\p{N}/u.test(ch)) {
      slug += ch;
      lastWasHyphen = false;
    } else if (!lastWasHyphen) {
      slug += '-';
      lastWasHyphen = true;
    }
  }

  slug = slug.replace(/-+$/, '');

  if (slug.length === 0) {
    return 'untitled';
  }

  const truncated = [...slug].slice(0, MAX_SLUG_LEN).join('');

  return truncated.replace(/-+$/, '');
}
