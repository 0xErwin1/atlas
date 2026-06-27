/**
 * Up-to-two-letter initials for an avatar fallback. Uses the first letter of the
 * first two whitespace-separated words when there are at least two, otherwise the
 * first two characters of the trimmed name. Falls back to `?` for empty input.
 */
export function initials(name: string | null | undefined): string {
  const base = (name ?? '').trim() || '?';
  const parts = base.split(/\s+/).filter(Boolean);
  const first = parts[0];
  const second = parts[1];
  if (first && second) return (first.charAt(0) + second.charAt(0)).toUpperCase();
  return base.slice(0, 2).toUpperCase();
}

/**
 * Format an ISO timestamp as a short `Mon DD, YYYY` date. Returns `fallback`
 * (default `—`) when the value is absent, so callers pass their own empty label
 * (e.g. `Never` for a key that has never been used).
 */
export function formatDate(iso: string | null | undefined, fallback = '—'): string {
  if (!iso) return fallback;
  return new Date(iso).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: '2-digit' });
}
