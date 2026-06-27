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

/**
 * Human-readable byte size (e.g. `820 B`, `1.4 MB`). Uses binary (1024) steps and
 * one decimal below 10 of a unit, none above, so the label stays compact.
 */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;

  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let unit = 0;

  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }

  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}
